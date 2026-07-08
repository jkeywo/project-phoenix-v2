export class PhNavigationMap extends HTMLElement {
  #state = null;
  #canvas = null;
  #ctx = null;
  #offscreen = null;
  #rafId = null;
  #resizeObserver = null;
  #needsRender = true;
  #projectedBlips = [];
  #selectedBlip = null;
  #overlay = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; position: relative; }',
      'canvas { display: block; width: 100%; height: 100%; }',
      '#overlay {',
      '  position: absolute; bottom: 0; left: 0; right: 0;',
      '  padding: 10px 14px 14px;',
      '  background: linear-gradient(0deg, rgba(5,8,24,0.95) 0%, rgba(5,8,24,0.7) 70%, transparent 100%);',
      '  border-top: 1px solid rgba(40,50,80,0.45);',
      '  font-family: "JetBrains Mono", monospace;',
      '  pointer-events: none; display: none;',
      '}',
      '#overlay.show { display: block; }',
      '#overlay .ov-name { font-size: 13px; color: #cce; font-weight: 600; letter-spacing: 0.05em; }',
      '#overlay .ov-detail { font-size: 9px; color: #6a7178; margin-top: 3px; letter-spacing: 0.15em; display: flex; gap: 10px; }',
      '.st-hostile { color: #ff6040; }',
      '.st-friendly { color: #4ec870; }',
      '.st-neutral { color: #7a90c0; }',
      '.st-unknown { color: #6a7178; }',
      '</style>',
      '<div style="position:relative;width:100%;height:100%">',
      '  <canvas></canvas>',
      '  <div id="overlay">',
      '    <div class="ov-name" id="ov-name"></div>',
      '    <div class="ov-detail">',
      '      <span id="ov-kind"></span>',
      '      <span id="ov-stance" class="st-unknown"></span>',
      '    </div>',
      '  </div>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this.#canvas = this.shadowRoot.querySelector('canvas');
    this.#ctx = this.#canvas.getContext('2d', { alpha: false });
    this.#overlay = this.shadowRoot.getElementById('overlay');
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

  #worldToScreen(wx, wz, shipX, shipZ, headingRad, scale, cx, cy) {
    const dx = wx - shipX;
    const dz = wz - shipZ;
    const rx = dx * Math.cos(headingRad) + dz * Math.sin(headingRad);
    const rz = dx * Math.sin(headingRad) - dz * Math.cos(headingRad);
    return [cx + rx * scale, cy + rz * scale];
  }

  #screenToWorld(sx, sy, shipX, shipZ, headingRad, scale, cx, cy) {
    const nx = (sx - cx) / scale;
    const ny = (sy - cy) / scale;
    const cosH = Math.cos(headingRad);
    const sinH = Math.sin(headingRad);
    const wx = nx * cosH + ny * sinH + shipX;
    const wz = nx * sinH - ny * cosH + shipZ;
    return [wx, wz];
  }

  #render() {
    const canvas = this.#canvas;
    if (!canvas) return;
    const W = canvas.width, H = canvas.height;
    if (W === 0 || H === 0) return;
    const cx = W / 2, cy = H / 2;
    const R = Math.min(W, H) / 2;
    const state = this.#state || {};
    const blips = state.blips || [];
    const range = state.range || 50000;
    const rangeClamped = range > 0 ? range : 50000;
    const shipPos = state.ship_pos || { x: 0, z: 0 };
    const shipHeading = state.ship_heading || 0;
    const waypoint = state.waypoint || null;
    const headingRad = shipHeading * Math.PI / 180;
    const scale = R / rangeClamped;

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

    this.#drawGrid(octx, cx, cy, scale, shipPos.x, shipPos.z, headingRad, W, H, rangeClamped);

    if (waypoint && Number.isFinite(waypoint.x) && Number.isFinite(waypoint.z)) {
      this.#drawWaypoint(octx, waypoint, cx, cy, scale, shipPos, headingRad);
    }

    this.#drawShipMarker(octx, cx, cy, R);

    this.#projectedBlips = [];
    const blipR = Math.max(3, R * 0.015);
    for (const b of blips) {
      const [sx, sy] = this.#worldToScreen(b.x, b.z, shipPos.x, shipPos.z, headingRad, scale, cx, cy);
      if (sx < -50 || sx > W + 50 || sy < -50 || sy > H + 50) continue;
      const color = this.#blipColor(b.stance);
      this.#drawBlipShape(octx, b.kind, sx, sy, blipR, color);
      if (b.label) {
        octx.font = '10px "JetBrains Mono", monospace';
        octx.fillStyle = color;
        octx.fillText(b.label, sx + blipR + 4, sy + 4);
      }
      this.#projectedBlips.push({ uuid: b.uuid, sx, sy, hitR: Math.max(14, blipR + 6), blip: b });
    }

    if (this.#offscreen) {
      this.#ctx.drawImage(this.#offscreen, 0, 0);
    }

    this.#needsRender = false;
  }

  #drawGrid(octx, cx, cy, scale, shipX, shipZ, headingRad, W, H, range) {
    const cosH = Math.cos(headingRad);
    const sinH = Math.sin(headingRad);

    const toScreen = (wx, wz) => {
      const dx = wx - shipX;
      const dz = wz - shipZ;
      const rx = dx * cosH + dz * sinH;
      const rz = dx * sinH - dz * cosH;
      return [cx + rx * scale, cy + rz * scale];
    };

    const half = range * 1.5;
    const minWX = shipX - half;
    const maxWX = shipX + half;
    const minWZ = shipZ - half;
    const maxWZ = shipZ + half;

    let minor, major;
    if (range > 50000) { minor = 2000; major = 10000; }
    else if (range > 10000) { minor = 500; major = 2000; }
    else if (range > 2000) { minor = 100; major = 500; }
    else { minor = 50; major = 200; }

    octx.strokeStyle = 'rgba(40,58,120,0.18)';
    octx.lineWidth = 0.5;
    for (let gx = Math.ceil(minWX / minor) * minor; gx < maxWX; gx += minor) {
      const [x1, y1] = toScreen(gx, minWZ);
      const [x2, y2] = toScreen(gx, maxWZ);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }
    for (let gz = Math.ceil(minWZ / minor) * minor; gz < maxWZ; gz += minor) {
      const [x1, y1] = toScreen(minWX, gz);
      const [x2, y2] = toScreen(maxWX, gz);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }

    octx.strokeStyle = 'rgba(70,95,165,0.28)';
    octx.lineWidth = 0.8;
    for (let gx = Math.ceil(minWX / major) * major; gx < maxWX; gx += major) {
      const [x1, y1] = toScreen(gx, minWZ);
      const [x2, y2] = toScreen(gx, maxWZ);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }
    for (let gz = Math.ceil(minWZ / major) * major; gz < maxWZ; gz += major) {
      const [x1, y1] = toScreen(minWX, gz);
      const [x2, y2] = toScreen(maxWX, gz);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }
  }

  #drawShipMarker(octx, cx, cy, R) {
    const r = Math.max(6, R * 0.025);
    octx.save();
    octx.strokeStyle = 'rgba(108,182,208,0.2)';
    octx.lineWidth = 1;
    octx.beginPath(); octx.arc(cx, cy, r * 2.2, 0, Math.PI * 2); octx.stroke();
    octx.shadowColor = '#6cb6d0';
    octx.shadowBlur = 14;
    octx.fillStyle = '#6cb6d0';
    octx.beginPath();
    octx.moveTo(cx, cy - r);
    octx.lineTo(cx + r * 0.65, cy + r * 0.45);
    octx.lineTo(cx, cy + r * 0.05);
    octx.lineTo(cx - r * 0.65, cy + r * 0.45);
    octx.closePath();
    octx.fill();
    octx.shadowBlur = 0;
    octx.restore();
  }

  #drawWaypoint(octx, wp, cx, cy, scale, shipPos, headingRad) {
    const [px, py] = this.#worldToScreen(wp.x, wp.z, shipPos.x, shipPos.z, headingRad, scale, cx, cy);
    const d = 8;
    octx.save();
    octx.fillStyle = 'rgba(212,168,32,0.18)';
    octx.strokeStyle = '#d4a820';
    octx.lineWidth = 1.5;
    octx.shadowColor = '#d4a820';
    octx.shadowBlur = 10;
    octx.beginPath();
    octx.moveTo(px, py - d); octx.lineTo(px + d, py);
    octx.lineTo(px, py + d); octx.lineTo(px - d, py);
    octx.closePath();
    octx.fill(); octx.stroke();
    octx.shadowBlur = 0;
    octx.font = '8px "JetBrains Mono", monospace';
    octx.fillStyle = '#d4a820';
    octx.textAlign = 'left';
    octx.fillText('WP', px + d + 4, py + 3);
    octx.restore();
  }

  #blipColor(stance) {
    if (stance === 'hostile') return '#ff6040';
    if (stance === 'friendly') return '#4ec870';
    if (stance === 'neutral') return '#7a90c0';
    return '#a8b0c0';
  }

  #drawBlipShape(octx, kind, sx, sy, r, color) {
    octx.save();
    octx.fillStyle = color;
    if (kind === 'star') {
      const spikes = 5;
      const outerR = r * 1.4;
      const innerR = r * 0.6;
      octx.beginPath();
      for (let i = 0; i < spikes * 2; i++) {
        const angle = (i * Math.PI) / spikes - Math.PI / 2;
        const rad = i % 2 === 0 ? outerR : innerR;
        const px = sx + rad * Math.cos(angle);
        const py = sy + rad * Math.sin(angle);
        if (i === 0) octx.moveTo(px, py);
        else octx.lineTo(px, py);
      }
      octx.closePath(); octx.fill();
    } else if (kind === 'station') {
      const half = r * 1.1;
      octx.fillRect(sx - half, sy - half, half * 2, half * 2);
    } else if (kind === 'ship') {
      const s = r * 1.3;
      octx.beginPath();
      octx.moveTo(sx, sy - s);
      octx.lineTo(sx + s * 0.6, sy + s * 0.5);
      octx.lineTo(sx, sy + s * 0.05);
      octx.lineTo(sx - s * 0.6, sy + s * 0.5);
      octx.closePath(); octx.fill();
    } else if (kind === 'asteroid') {
      octx.beginPath();
      octx.arc(sx, sy, r * 0.5, 0, Math.PI * 2);
      octx.fill();
    } else {
      octx.beginPath();
      octx.arc(sx, sy, r, 0, Math.PI * 2);
      octx.fill();
    }
    octx.restore();
  }

  #showOverlay(blip) {
    const nameEl = this.shadowRoot.getElementById('ov-name');
    const kindEl = this.shadowRoot.getElementById('ov-kind');
    const stanceEl = this.shadowRoot.getElementById('ov-stance');
    if (!blip) {
      this.#overlay.classList.remove('show');
      return;
    }
    nameEl.textContent = blip.label || blip.uuid || 'UNKNOWN';
    kindEl.textContent = (blip.kind || 'unknown').toUpperCase();
    stanceEl.textContent = (blip.stance || 'UNKNOWN').toUpperCase();
    stanceEl.className = 'st-' + (blip.stance || 'unknown');
    this.#overlay.classList.add('show');
  }

  #getBlipAt(canvasX, canvasY) {
    let best = null;
    let bestDist = Infinity;
    for (const b of this.#projectedBlips) {
      const dist = Math.hypot(canvasX - b.sx, canvasY - b.sy);
      if (dist <= b.hitR && dist < bestDist) {
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
    const canvasX = x * scaleX;
    const canvasY = y * scaleY;

    const hit = this.#getBlipAt(canvasX, canvasY);
    const state = this.#state || {};
    const shipPos = state.ship_pos || { x: 0, z: 0 };
    const range = state.range || 50000;
    const rangeClamped = range > 0 ? range : 50000;
    const shipHeading = state.ship_heading || 0;
    const headingRad = shipHeading * Math.PI / 180;
    const R = Math.min(this.#canvas.width, this.#canvas.height) / 2;
    const scale = R / rangeClamped;
    const cx = this.#canvas.width / 2;
    const cy = this.#canvas.height / 2;

    const [wx, wz] = this.#screenToWorld(canvasX, canvasY, shipPos.x, shipPos.z, headingRad, scale, cx, cy);

    if (hit && this.sendAction) {
      this.#selectedBlip = hit.blip;
      this.#showOverlay(hit.blip);
      this.sendAction('set_waypoint', { x: hit.blip.x, z: hit.blip.z, entity_uuid: hit.uuid });
    } else if (this.sendAction) {
      this.#selectedBlip = null;
      this.#showOverlay(null);
      this.sendAction('set_waypoint', { x: wx, z: wz });
    }
  }

  #boundTap = (e) => this.#onPointerTap(e);
}

if (typeof window !== 'undefined' && !customElements.get('ph-navigation-map')) {
  customElements.define('ph-navigation-map', PhNavigationMap);
}
