export class PhRadar extends HTMLElement {
  #state = null;
  #canvas = null;
  #ctx = null;
  #offscreen = null;
  #rafId = null;
  #resizeObserver = null;
  #needsRender = true;
  #icons = {};
  #projectedBlips = [];

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; }',
      'canvas { display: block; width: 100%; height: 100%; }',
      '</style>',
      '<canvas></canvas>',
    ].join('\n');
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this.#canvas = this.shadowRoot.querySelector('canvas');
    this.#ctx = this.#canvas.getContext('2d', { alpha: false });
    this.#initResize();
    this.#rafLoop();
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    this.#canvas.addEventListener('click', this.#boundTap);
    this.#canvas.addEventListener('touchstart', this.#boundTap, { passive: false });
  }

  disconnectedCallback() {
    if (this.#rafId != null) {
      cancelAnimationFrame(this.#rafId);
      this.#rafId = null;
    }
    if (this.#resizeObserver) {
      this.#resizeObserver.disconnect();
      this.#resizeObserver = null;
    }
    this.#canvas.removeEventListener('click', this.#boundTap);
    this.#canvas.removeEventListener('touchstart', this.#boundTap);
  }

  set state(val) {
    this.#state = val;
    this.#needsRender = true;
  }

  get state() { return this.#state; }

  #initResize() {
    const updateSize = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = this.getBoundingClientRect();
      const newW = Math.round(rect.width * dpr);
      const newH = Math.round(rect.height * dpr);
      if (newW > 0 && newH > 0 && (this.#canvas.width !== newW || this.#canvas.height !== newH)) {
        this.#canvas.width = newW;
        this.#canvas.height = newH;
        this.#needsRender = true;
      }
    };
    updateSize();
    this.#resizeObserver = new ResizeObserver(() => updateSize());
    this.#resizeObserver.observe(this);
  }

  #rafLoop() {
    if (this.#needsRender) this.#render();
    this.#rafId = requestAnimationFrame(() => this.#rafLoop());
  }

  #iconStemFromName(name) {
    if (!name) return '';
    return name.charAt(0).toUpperCase() + name.slice(1);
  }

  #getIconImage(name) {
    if (!name) return null;
    if (this.#icons[name]) return this.#icons[name];
    if (typeof Image === 'undefined') return null;
    const img = new Image();
    img.onload = () => { this.#needsRender = true; };
    const stem = name === 'player' ? 'PlayerShip' : this.#iconStemFromName(name);
    img.src = '../../assets/radar_icons/Icon-' + stem + '.png';
    this.#icons[name] = img;
    return img;
  }

  #render() {
    const canvas = this.#canvas;
    if (!canvas) return;
    const W = canvas.width, H = canvas.height;
    if (W === 0 || H === 0) return;
    const cx = W / 2, cy = H / 2;
    const R = Math.min(W, H) / 2;
    const state = this.#state || {};

    let octx = this.#ctx;
    if (typeof document !== 'undefined') {
      if (!this.#offscreen || this.#offscreen.width !== W || this.#offscreen.height !== H) {
        this.#offscreen = document.createElement('canvas');
        this.#offscreen.width = W;
        this.#offscreen.height = H;
      }
      octx = this.#offscreen.getContext('2d');
    }

    octx.fillStyle = '#07080c';
    octx.fillRect(0, 0, W, H);

    octx.fillStyle = 'rgba(5,8,22,0.52)';
    octx.beginPath();
    octx.arc(cx, cy, R, 0, Math.PI * 2);
    octx.fill();

    if (this.#offscreen) {
      this.#ctx.drawImage(this.#offscreen, 0, 0);
    }

    const blips = state.blips || [];
    if (blips.length === 0) { this.#needsRender = false; return; }

    this.#projectedBlips = [];

    for (const b of blips) {
      const bx = cx + (b.radar_x != null ? b.radar_x : 0) * R;
      const by = cy - (b.radar_y != null ? b.radar_y : 0) * R;
      const dotR = Math.max(6, (b.scaled_radius || 0) * R * 0.6);
      const color = b.color || '#a8b0c0';

      if (b.kind === 'waypoint') {
        this.#drawTargetBlip(octx, bx, by, dotR, !!b.edge, '#d4a820');
      } else if (b.kind === 'tactical-target') {
        this.#drawTargetBlip(octx, bx, by, dotR, !!b.edge, '#ff3344');
      } else if (b.kind === 'science-target') {
        this.#drawTargetBlip(octx, bx, by, dotR, !!b.edge, '#3399ff');
      } else {
        const iconName = b.icon;
        const icon = iconName ? this.#getIconImage(iconName) : null;
        const iconLoaded = icon && icon.complete && icon.naturalWidth > 0;

        if (iconLoaded) {
          const size = dotR * 2;
          octx.drawImage(icon, bx - dotR, by - dotR, size, size);
        } else {
          octx.beginPath();
          octx.arc(bx, by, dotR, 0, Math.PI * 2);
          octx.fillStyle = color;
          octx.fill();
        }
      }

      if (b.label) {
        octx.font = '11px "JetBrains Mono", monospace';
        octx.fillStyle = 'rgba(153,255,217,0.9)';
        octx.fillText(b.label, bx + dotR + 4, by + 4);
      }

      if (state.selected_target_uuid && state.selected_target_uuid === b.uuid) {
        this.#drawRing(octx, bx, by, dotR + 6, 2, '#5fd8e8');
      }
      if (state.target_uuid && state.target_uuid === b.uuid) {
        this.#drawRing(octx, bx, by, dotR + 8, 2, '#ff3344');
      }

      this.#projectedBlips.push({ uuid: b.uuid, bx, by, dotR });
    }

    if (this.#offscreen) {
      this.#ctx.drawImage(this.#offscreen, 0, 0);
    }

    this.#needsRender = false;
  }

  #drawRing(ctx, x, y, r, lineWidth, color) {
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.strokeStyle = color;
    ctx.lineWidth = lineWidth;
    ctx.stroke();
  }

  #drawTargetBlip(ctx, bx, by, dotR, edge, color) {
    const r = Math.max(7, dotR + 3);
    ctx.save();
    ctx.translate(bx, by);
    ctx.strokeStyle = color;
    ctx.lineWidth = edge ? 2.5 : 2;

    ctx.beginPath();
    ctx.moveTo(0, -r);
    ctx.lineTo(r, 0);
    ctx.lineTo(0, r);
    ctx.lineTo(-r, 0);
    ctx.closePath();
    ctx.globalAlpha = edge ? 0.15 : 0.26;
    ctx.fillStyle = color;
    ctx.fill();
    ctx.globalAlpha = 1.0;
    ctx.stroke();

    ctx.beginPath();
    ctx.arc(0, 0, r + 5, 0, Math.PI * 2);
    ctx.stroke();

    if (edge) {
      ctx.beginPath();
      ctx.moveTo(0, -r - 8);
      ctx.lineTo(4, -r - 1);
      ctx.lineTo(-4, -r - 1);
      ctx.closePath();
      ctx.fillStyle = color;
      ctx.fill();
    }
    ctx.restore();
  }

  #getBlipAt(canvasX, canvasY) {
    const blips = this.#projectedBlips || [];
    let best = null;
    let bestDist = Infinity;
    for (const b of blips) {
      const hitR = Math.max(14, b.dotR + 6);
      const dist = Math.hypot(canvasX - b.bx, canvasY - b.by);
      if (dist <= hitR && dist < bestDist) {
        best = b;
        bestDist = dist;
      }
    }
    return best;
  }

  #onPointerTap(e) {
    if (!this.#canvas) return;
    const rect = this.#canvas.getBoundingClientRect();
    const touch = e.touches ? e.touches[0] : e;
    const x = touch.clientX - rect.left;
    const y = touch.clientY - rect.top;
    const scaleX = this.#canvas.width / rect.width;
    const scaleY = this.#canvas.height / rect.height;
    const blip = this.#getBlipAt(x * scaleX, y * scaleY);
    if (blip && this.sendAction) {
      this.sendAction('set_target', { uuid: blip.uuid });
    }
  }

  #boundTap = (e) => this.#onPointerTap(e);
}

if (typeof window !== 'undefined' && !customElements.get('ph-radar')) {
  customElements.define('ph-radar', PhRadar);
}
