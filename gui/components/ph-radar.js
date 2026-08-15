export class PhRadar extends HTMLElement {
  #state = null;
  #canvas = null;
  #ctx = null;
  #offscreen = null;
  #rafId = null;
  #resizeObserver = null;
  #needsRender = true;
  #icons = {};
  #backgroundImage = null;
  #surroundImage = null;
  #projectedBlips = [];

  // Buffer pixels per CSS pixel — `canvas.width / rect.width`, i.e. the device
  // pixel ratio in practice.
  //
  // The backing store is deliberately sized rect × devicePixelRatio so the
  // scope rasterises crisply on a phone (see #initResize). Everything the
  // scope DRAWS was then authored as a bare number — an 11px label font, a
  // 6px minimum blip radius, a 14px tap radius — and a bare number in a
  // buffer-space context is a DEVICE pixel. On a 3× phone that renders the
  // labels at 3.7 CSS px and shrinks the tap target to under 5, which is the
  // radar half of PRD #1023's problem statement.
  //
  // So every fixed size below is multiplied by this. Geometry derived from R
  // (blip positions, `scaled_radius`) is already proportional to the buffer
  // and must NOT be scaled again. gui/components/ph-navigation-map.js reached
  // the same conclusion independently for its label fonts.
  #px = 1;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = [
      '<style>',
      ':host { display: block; }',
      'canvas { display: block; width: 100%; height: 100%; }',
      '</style>',
      '<canvas></canvas>',
    ].join('\n');
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    this.#canvas = this.shadowRoot.querySelector('canvas');
    this.#ctx = this.#canvas.getContext('2d', { alpha: false });
    this.#backgroundImage = this.#loadImage('../../assets/helm_console/radar-bg.png');
    this.#surroundImage = this.#loadImage('../../assets/helm_console/radar-surround.png');
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

  #loadImage(src) {
    if (typeof Image === 'undefined') return null;
    const img = new Image();
    img.onload = () => { this.#needsRender = true; };
    img.src = src;
    return img;
  }

  #imageIsLoaded(img) {
    return img && img.complete && img.naturalWidth > 0;
  }

  #render() {
    const canvas = this.#canvas;
    if (!canvas) return;
    const W = canvas.width, H = canvas.height;
    if (W === 0 || H === 0) return;
    const cx = W / 2, cy = H / 2;
    const R = Math.min(W, H) / 2;
    const state = this.#state || {};

    // Refreshed per frame: a phone dragged onto an external display changes
    // devicePixelRatio without changing the element's CSS size.
    const rect = this.getBoundingClientRect ? this.getBoundingClientRect() : null;
    this.#px = (rect && rect.width > 0) ? (W / rect.width) : 1;
    const px = this.#px;

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

    // The surround is fixed console chrome. The radar screen is the rotating,
    // ship-relative backdrop; blips remain above both layers.
    if (this.#imageIsLoaded(this.#surroundImage)) {
      octx.drawImage(this.#surroundImage, 0, 0, W, H);
    }

    if (this.#imageIsLoaded(this.#backgroundImage)) {
      const heading = state.ship_heading || 0;
      octx.save();
      octx.translate(cx, cy);
      octx.rotate(-heading * Math.PI / 180);
      octx.drawImage(this.#backgroundImage, -R, -R, R * 2, R * 2);
      octx.restore();
    } else {
      octx.fillStyle = 'rgba(5,8,22,0.52)';
      octx.beginPath();
      octx.arc(cx, cy, R, 0, Math.PI * 2);
      octx.fill();
    }

    if (this.#offscreen) {
      this.#ctx.drawImage(this.#offscreen, 0, 0);
    }

    const blips = state.blips || [];
    if (blips.length === 0) { this.#needsRender = false; return; }

    this.#projectedBlips = [];

    for (const b of blips) {
      const bx = cx + (b.radar_x != null ? b.radar_x : 0) * R;
      const by = cy - (b.radar_y != null ? b.radar_y : 0) * R;
      const dotR = Math.max(6 * px, (b.scaled_radius || 0) * R * 0.6);
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
        octx.font = Math.round(11 * px) + 'px "JetBrains Mono", monospace';
        octx.fillStyle = 'rgba(153,255,217,0.9)';
        octx.fillText(b.label, bx + dotR + 4 * px, by + 4 * px);
      }

      if (state.selected_target_uuid && state.selected_target_uuid === b.uuid) {
        this.#drawRing(octx, bx, by, dotR + 6 * px, 2 * px, '#5fd8e8');
      }
      if (state.target_uuid && state.target_uuid === b.uuid) {
        this.#drawRing(octx, bx, by, dotR + 8 * px, 2 * px, '#ff3344');
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
    const px = this.#px;
    const r = Math.max(7 * px, dotR + 3 * px);
    ctx.save();
    ctx.translate(bx, by);
    ctx.strokeStyle = color;
    ctx.lineWidth = (edge ? 2.5 : 2) * px;

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
    ctx.arc(0, 0, r + 5 * px, 0, Math.PI * 2);
    ctx.stroke();

    if (edge) {
      ctx.beginPath();
      ctx.moveTo(0, -r - 8 * px);
      ctx.lineTo(4 * px, -r - 1 * px);
      ctx.lineTo(-4 * px, -r - 1 * px);
      ctx.closePath();
      ctx.fillStyle = color;
      ctx.fill();
    }
    ctx.restore();
  }

  #getBlipAt(canvasX, canvasY) {
    const blips = this.#projectedBlips || [];
    const px = this.#px;
    let best = null;
    let bestDist = Infinity;
    for (const b of blips) {
      // Buffer-space coordinates come in from #onPointerTap, so the floor is
      // a CSS-pixel floor scaled up — a fixed 14 would be under 5 CSS px of
      // finger on a 3× phone.
      const hitR = Math.max(14 * px, b.dotR + 6 * px);
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
