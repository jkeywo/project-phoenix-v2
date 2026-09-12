// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the scale readout never draws against an empty table.
// No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { phColor } from './ph-console-styles.js';
import {
  ringPlan, scaleReadout, phPx, TEXT_MIN_FALLBACK_PX, textScaleOf, forcedColorsActive,
} from './ph-scope-chrome.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * How far a label sits from its blip, and how far apart two labels must stay.
 *
 * Both in CSS pixels, both multiplied by `#px` at the draw call — see the note
 * on that field.
 */
const LABEL_GAP_CSS = 4;
const LABEL_HALO_CSS = 3;

/** The gap between a lock's two concentric rings — see the render loop. */
const LOCK_RING_GAP_CSS = 4;

export class PhRadar extends PhElement {
  // The state backing stays PRIVATE: `set state` is overridden below to flag a
  // redraw rather than paint synchronously (the base default paints on assign),
  // and nothing touches `#state` during `onTemplate()` — the first draw is
  // deferred to a rAF, so this field is always installed before it is read.
  #state = null;
  #offscreen = null;
  #icons = {};
  #projectedBlips = [];

  // Buffer pixels per CSS pixel — `canvas.width / rect.width`, i.e. the device
  // pixel ratio in practice.
  //
  // The backing store is deliberately sized rect × devicePixelRatio so the
  // scope rasterises crisply on a phone (see initResize). Everything the
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

  // The operator's text-scale multiplier as of the last frame this scope
  // actually painted. Read once per rAF tick (see `#rafLoop`) so a live
  // change — dragging the settings slider while this console is open —
  // reaches the canvas the same frame it reaches every rem-based string,
  // rather than waiting for some UNRELATED state push to next set
  // `needsRender`. A CSS custom property changing is not itself a DOM
  // mutation this element observes, so nothing else would notice.
  #lastTextScale = NaN;

  template() {
    return [
      '<style>',
      ':host { display: block; }',
      'canvas { display: block; width: 100%; height: 100%; }',
      '</style>',
      '<canvas></canvas>',
    ].join('\n');
  }

  onTemplate() {
    // Canvas refs and the render bookkeeping live as PLAIN properties: this runs
    // inside the base constructor, BEFORE this subclass's field-init and private
    // methods are installed, so anything the setup or first draw touches must be
    // a plain property, never a declared #field (see ph-element.js's field-init
    // note). The state backing and the pure draw-scratch fields above stay
    // private because nothing reads them until the deferred first frame.
    this.canvas = this.shadowRoot.querySelector('canvas');
    this.ctx = this.canvas.getContext('2d', { alpha: false });
    this.needsRender = true;
    this.rafId = null;
    this.resizeObserver = null;
    this.backgroundImage = this.loadImage('../../assets/helm_console/radar-bg.png');
    this.surroundImage = this.loadImage('../../assets/helm_console/radar-surround.png');
    this.initResize();
    // Defer the first frame: the arrow's `this.#rafLoop` is resolved when the
    // rAF fires (after construction, once the private method exists), not now.
    // The synchronous first tick the constructor used to run was a no-op anyway
    // — the canvas is 0×0 until laid out — so scheduling it loses nothing.
    this.rafId = requestAnimationFrame(() => this.#rafLoop());
  }

  connectedCallback() {
    super.connectedCallback();
    this.canvas.addEventListener('click', this.#boundTap);
    this.canvas.addEventListener('touchstart', this.#boundTap, { passive: false });
  }

  disconnectedCallback() {
    if (this.rafId != null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
    if (this.resizeObserver) {
      this.resizeObserver.disconnect();
      this.resizeObserver = null;
    }
    this.canvas.removeEventListener('click', this.#boundTap);
    this.canvas.removeEventListener('touchstart', this.#boundTap);
  }

  set state(val) {
    this.#state = val;
    this.needsRender = true;
  }

  get state() { return this.#state; }

  initResize() {
    const updateSize = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = this.getBoundingClientRect();
      const newW = Math.round(rect.width * dpr);
      const newH = Math.round(rect.height * dpr);
      if (newW > 0 && newH > 0 && (this.canvas.width !== newW || this.canvas.height !== newH)) {
        this.canvas.width = newW;
        this.canvas.height = newH;
        this.needsRender = true;
      }
    };
    updateSize();
    this.resizeObserver = new ResizeObserver(() => updateSize());
    this.resizeObserver.observe(this);
  }

  #rafLoop() {
    const scale = textScaleOf(this);
    if (scale !== this.#lastTextScale) {
      this.#lastTextScale = scale;
      this.needsRender = true;
    }
    if (this.needsRender) this.#render();
    this.rafId = requestAnimationFrame(() => this.#rafLoop());
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
    img.onload = () => { this.needsRender = true; };
    const stem = name === 'player' ? 'PlayerShip' : this.#iconStemFromName(name);
    img.src = '../../assets/radar_icons/Icon-' + stem + '.png';
    this.#icons[name] = img;
    return img;
  }

  loadImage(src) {
    if (typeof Image === 'undefined') return null;
    const img = new Image();
    img.onload = () => { this.needsRender = true; };
    img.src = src;
    return img;
  }

  #imageIsLoaded(img) {
    return img && img.complete && img.naturalWidth > 0;
  }

  #render() {
    const canvas = this.canvas;
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

    let octx = this.ctx;
    if (typeof document !== 'undefined') {
      if (!this.#offscreen || this.#offscreen.width !== W || this.#offscreen.height !== H) {
        this.#offscreen = document.createElement('canvas');
        this.#offscreen.width = W;
        this.#offscreen.height = H;
      }
      octx = this.#offscreen.getContext('2d');
    }

    octx.fillStyle = phColor(this, 'var(--surface-abyss)');
    octx.fillRect(0, 0, W, H);

    // The surround is fixed console chrome. The radar screen is the rotating,
    // ship-relative backdrop; blips remain above both layers.
    if (this.#imageIsLoaded(this.surroundImage)) {
      octx.drawImage(this.surroundImage, 0, 0, W, H);
    }

    if (this.#imageIsLoaded(this.backgroundImage)) {
      const heading = state.ship_heading || 0;
      octx.save();
      octx.translate(cx, cy);
      octx.rotate(-heading * Math.PI / 180);
      octx.drawImage(this.backgroundImage, -R, -R, R * 2, R * 2);
      octx.restore();
    } else {
      octx.fillStyle = phColor(this, 'rgba(var(--rgb-deep), 0.52)');
      octx.beginPath();
      octx.arc(cx, cy, R, 0, Math.PI * 2);
      octx.fill();
    }

    // Rings before contacts, so a blip is never drawn under its own scale.
    this.#drawRangeRings(octx, cx, cy, R, px, state.range);

    if (this.#offscreen) {
      this.ctx.drawImage(this.#offscreen, 0, 0);
    }

    const blips = state.blips || [];
    if (blips.length === 0) { this.needsRender = false; return; }

    // Queried live, once per frame (issue #1424): a real OS/browser toggle a
    // player can flip while Phoenix is running, not a Phoenix setting cached
    // across renders. See forcedColorsActive()'s own note on why canvas needs
    // this at all when tokens.css already covers every DOM-styled control.
    const forced = forcedColorsActive();

    this.#projectedBlips = [];
    const labels = [];

    for (const b of blips) {
      const bx = cx + (b.radar_x != null ? b.radar_x : 0) * R;
      const by = cy - (b.radar_y != null ? b.radar_y : 0) * R;
      const dotR = Math.max(6 * px, (b.scaled_radius || 0) * R * 0.6);
      const color = b.color || 'var(--ink-dim)';

      if (b.kind === 'waypoint') {
        this.#drawTargetBlip(octx, bx, by, dotR, !!b.edge, 'var(--gold)');
      } else if (b.kind === 'tactical-target') {
        this.#drawTargetBlip(octx, bx, by, dotR, !!b.edge, 'var(--fire-hot)');
      } else if (b.kind === 'science-target') {
        this.#drawTargetBlip(octx, bx, by, dotR, !!b.edge, 'var(--science)');
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
          octx.fillStyle = phColor(this, color);
          octx.fill();
        }

        // A hostile contact's marker (issue #1424): an icon loaded from
        // `assets/radar_icons/` is a bare silhouette by CLASS (battleship,
        // cruiser, ship…), never by STANCE — so a hostile heavy and a friendly
        // one of the same class drew IDENTICALLY once the icon loaded, colour
        // tint included: `#color` above only ever reaches the fallback circle
        // an unloaded icon takes for one frame. `target_tags` (not the
        // "purely cosmetic" `threat_level` — src/entities/target.rs's own
        // words — which describes a scanned target's flavour text, not IFF)
        // is the actual stance a console's `selects` filter already reads, so
        // it is the honest source for this marker too. Shape carries the
        // meaning — present or absent, never a colour swap alone — so it
        // survives both forced colours and a colour-deficient eye.
        if (this.#isHostile(b)) this.#drawHostileMarker(octx, bx, by, dotR, px, forced);
      }

      // Collected, not drawn: a label's final Y depends on the other labels,
      // so nothing can be painted until every one of them is known.
      if (b.label) {
        labels.push({
          text: b.label,
          x: bx + dotR + LABEL_GAP_CSS * px,
          y: by + LABEL_GAP_CSS * px,
        });
      }

      // Selected and locked are both rendered here, and are deliberately NOT
      // colour-only distinctions of each other (issue #1424): a selected
      // candidate is one SINGLE ring; a Tactical lock is committed weapons
      // authority and draws as a DOUBLE ring — a shape difference that reads
      // the same in grayscale as it does in colour, on top of (not instead
      // of) the signal/fire-hot colour split already there. Sensors never
      // sets `target_uuid` (see ph-sensor-radar.js), so its own selection
      // never grows the second ring.
      if (state.selected_target_uuid && state.selected_target_uuid === b.uuid) {
        this.#drawRing(octx, bx, by, dotR + 6 * px, 2 * px, forced ? 'Highlight' : 'var(--signal)');
      }
      if (state.target_uuid && state.target_uuid === b.uuid) {
        const lockColor = forced ? 'Mark' : 'var(--fire-hot)';
        this.#drawRing(octx, bx, by, dotR + 8 * px, 2 * px, lockColor);
        this.#drawRing(octx, bx, by, dotR + 8 * px + LOCK_RING_GAP_CSS * px, 1.5 * px, lockColor);
      }

      this.#projectedBlips.push({ uuid: b.uuid, bx, by, dotR });
    }

    // Selected-target trajectory projection (issue #1339). Drawn after the
    // blips so the marker trail isn't hidden under an icon it happens to
    // pass behind; absent/`null` when the Science Target's velocity is
    // unknown, so a stationary or non-ship contact draws nothing here.
    this.#drawProjection(octx, cx, cy, R, px, state.target_projection);

    this.#drawLabels(octx, labels, px);

    if (this.#offscreen) {
      this.ctx.drawImage(this.#offscreen, 0, 0);
    }

    this.needsRender = false;
  }

  /**
   * The type floor in BUFFER pixels — `--text-min`, scaled for the backing
   * store AND for the operator's chosen text scale (PRD #1418 story 3; see
   * `textScaleOf`). The scope's contacts, rings and grid stay exactly where
   * they are — this only grows the TEXT drawn over them.
   */
  #labelFontPx(px) {
    return Math.round(phPx(this, '--text-min', TEXT_MIN_FALLBACK_PX) * px * textScaleOf(this));
  }

  /**
   * Range rings, and the scale readout that says what the outer one means.
   *
   * Harvested from gui/radar-widget.js, which drew three rings at a fixed 33 /
   * 66 / 100 % of the scope radius and labelled none of them. Those rings were
   * decoration: the middle one stood for two-thirds of whatever that console's
   * range happened to be, so the same picture meant 333 units on the helm scope
   * and 200 on the weapons scope, and nothing on screen said which.
   *
   * Here the radii come from `state.range` through `ringPlan`, which picks a
   * round spacing — so a ring is a distance a player can name, and the scale
   * readout names the outermost one. `range` is the LIVE per-tick value: it is
   * `bb.radar_range` on the helm blackboard, which shrinks as the radar system
   * takes damage and moves when doctrine changes it. The scope therefore
   * redraws its own scale rather than lying at the range it booted with.
   *
   * Handed no range, the rings fall back to the harvested thirds and no readout
   * is drawn — an unlabelled ring is a weaker picture than a labelled one, but
   * a labelled ring with no distance behind the label would be a false one.
   */
  #drawRangeRings(ctx, cx, cy, R, px, range) {
    const plan = ringPlan(range);
    const fractions = plan.length > 0
      ? plan.map((ring) => ring.fraction)
      : [0.33, 0.66, 1.0];

    ctx.save();
    ctx.strokeStyle = phColor(this, 'rgba(var(--rgb-edge-strong), 0.28)');
    ctx.lineWidth = Math.max(1, px);
    for (const fraction of fractions) {
      ctx.beginPath();
      ctx.arc(cx, cy, R * fraction, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.restore();

    const readout = plan.length > 0 ? scaleReadout(range) : '';
    if (!readout) return;

    // Against the outer ring on the AFT-PORT diagonal — the bottom-left — and
    // reading outwards from it, where a contact clamped to the rim is least
    // likely to be. The same reasoning that put the three corner readouts in
    // corners, with one correction (issue #1375).
    //
    // It used to sit on the aft-starboard diagonal, which is the corner ON
    // SCREEN now occupies. That button is ~90px of near-opaque chrome anchored
    // 6% in from the bottom-right, so on a 350px scope its inner corner reaches
    // to radius 128 of 175 — comfortably over the 0.707R point the readout was
    // painted at, and the scale simply disappeared underneath it.
    //
    // Aft-PORT shares its corner with the speed readout instead, and the two
    // provably clear each other at every scope size: the corner label's top
    // edge sits at 0.94S − lineHeight and this baseline at 0.8536S, so the gap
    // is 0.0864S − 10 CSS px, positive for any scope wider than 116px.
    const font = this.#labelFontPx(px);
    const diagonal = Math.SQRT1_2;
    ctx.save();
    ctx.font = font + 'px ' + this.#labelFontFamily();
    ctx.textAlign = 'left';
    ctx.textBaseline = 'bottom';
    this.#paintHaloed(
      ctx, readout,
      cx - R * diagonal + LABEL_GAP_CSS * px,
      cy + R * diagonal - LABEL_GAP_CSS * px,
      px, 'rgba(var(--rgb-edge-strong), 0.95)',
    );
    ctx.restore();
  }

  #labelFontFamily() {
    return '"JetBrains Mono", monospace';
  }

  /**
   * Draw contact labels: haloed, and nudged apart where they would overprint.
   *
   * TWO separate legibility problems, and they are not the same problem.
   *
   * A halo is about the BACKGROUND. The scope's backdrop is a photographic
   * PNG — a starfield with an asteroid belt across it — so a label's contrast
   * against it is whatever pixels it happens to land on. A dark outline behind
   * the glyphs makes the text readable over any of them, which is what a chart
   * does and what the flat `fillText` this replaces did not.
   *
   * De-collision is about the OTHER LABELS. Contacts cluster — that is what a
   * furball is — and two labels at the same Y overprint into an unreadable
   * smear exactly when the officer most needs to read them. The pass below
   * places labels top-down and pushes each one clear of the box already taken,
   * which keeps a label near its own blip (the offset only ever grows by as
   * much as the overlap) and is stable frame to frame because the order is a
   * function of position, not of arrival.
   */
  #drawLabels(ctx, labels, px) {
    if (labels.length === 0) return;

    const font = this.#labelFontPx(px);
    ctx.save();
    ctx.font = font + 'px ' + this.#labelFontFamily();
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';

    const lineHeight = font * 1.2;
    const placed = [];
    // Top-down, so a run of collisions cascades downward instead of
    // ping-ponging: each label only ever moves away from the ones above it.
    const ordered = labels.slice().sort((a, b) => a.y - b.y || a.x - b.x);

    for (const label of ordered) {
      const width = this.#measureText(ctx, label.text, font);
      let y = label.y;
      // Repeat until clear: pushing below one label can slide this one onto
      // another, and a single pass would leave that second overlap standing.
      for (let guard = 0; guard < placed.length + 1; guard += 1) {
        const clash = placed.find((box) => (
          label.x < box.x + box.width
          && label.x + width > box.x
          && y - lineHeight < box.y
          && y > box.y - lineHeight
        ));
        if (!clash) break;
        y = clash.y + lineHeight;
      }
      placed.push({ x: label.x, y, width });
      this.#paintHaloed(ctx, label.text, label.x, y, px,
        'rgba(var(--rgb-loaded-bright), 0.9)');
    }

    ctx.restore();
  }

  /**
   * Text with a dark outline behind it.
   *
   * `strokeText` before `fillText`, so the outline is centred on the glyph
   * edges and half of it ends up under the fill — a halo, rather than a stroked
   * outline drawn on top of the letters.
   */
  #paintHaloed(ctx, text, x, y, px, fill) {
    if (typeof ctx.strokeText === 'function') {
      ctx.lineWidth = LABEL_HALO_CSS * px;
      ctx.lineJoin = 'round';
      ctx.miterLimit = 2;
      ctx.strokeStyle = phColor(this, 'rgba(var(--rgb-void), 0.85)');
      ctx.strokeText(text, x, y);
    }
    ctx.fillStyle = phColor(this, fill);
    ctx.fillText(text, x, y);
  }

  /**
   * Label width in buffer pixels.
   *
   * `measureText` where the context has it. Where it does not, a monospace
   * estimate — the label font IS monospace, so 0.6 em per character is close
   * enough to keep the de-collision pass working rather than silently doing
   * nothing.
   */
  #measureText(ctx, text, font) {
    if (typeof ctx.measureText === 'function') {
      const m = ctx.measureText(text);
      if (m && Number.isFinite(m.width) && m.width > 0) return m.width;
    }
    return String(text).length * font * 0.6;
  }

  /**
   * Selected-target trajectory projection markers (issue #1339).
   *
   * Small ticks along the Science Target's future relative path, fading with
   * distance into the future so the near-term markers (where a manoeuvre
   * decision actually gets made) read strongest. `markers` is `null`/absent
   * whenever the target's velocity is unknown — no selection, a non-ship
   * contact, or an unresolvable target — in which case nothing is drawn.
   */
  #drawProjection(ctx, cx, cy, R, px, markers) {
    if (!markers || markers.length === 0) return;
    const dotR = 2.5 * px;
    ctx.save();
    ctx.fillStyle = phColor(this, 'var(--science)');
    markers.forEach((m, i) => {
      const bx = cx + (m.radar_x != null ? m.radar_x : 0) * R;
      const by = cy - (m.radar_y != null ? m.radar_y : 0) * R;
      // Fades from ~0.85 at the nearest marker to ~0.25 at the furthest.
      ctx.globalAlpha = Math.max(0.25, 0.85 - (i / markers.length) * 0.6);
      ctx.beginPath();
      ctx.arc(bx, by, dotR, 0, Math.PI * 2);
      ctx.fill();
    });
    ctx.restore();
  }

  #drawRing(ctx, x, y, r, lineWidth, color) {
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.strokeStyle = phColor(this, color);
    ctx.lineWidth = lineWidth;
    ctx.stroke();
  }

  /**
   * `target_tags` carries the entity's real IFF stance — the same list a
   * console's `selects` filter already reads (see `buildBlips` in
   * gui/console-state.js) — lower-cased for comparison since the wire does
   * not promise a case.
   */
  #isHostile(b) {
    const tags = b && b.target_tags;
    if (!Array.isArray(tags)) return false;
    return tags.some((tag) => String(tag).toLowerCase() === 'hostile');
  }

  /**
   * A small filled wedge above a hostile contact (issue #1424): SHAPE, not
   * colour, is what marks a contact hostile — the wedge is either drawn or it
   * is not, which survives forced colours and colour-deficient vision alike.
   * Skipped for the synthetic 'tactical-target' / 'science-target' / 'waypoint'
   * kinds (see the call site): those already draw their own distinct diamond
   * shape and are never `target_tags`-bearing world entities in the first
   * place.
   */
  #drawHostileMarker(ctx, bx, by, dotR, px, forced) {
    const r = dotR + 4 * px;
    const half = 3 * px;
    ctx.save();
    ctx.beginPath();
    ctx.moveTo(bx, by - r - half);
    ctx.lineTo(bx + half, by - r + half);
    ctx.lineTo(bx - half, by - r + half);
    ctx.closePath();
    ctx.fillStyle = phColor(this, forced ? 'Mark' : 'var(--fire-hot)');
    ctx.fill();
    ctx.restore();
  }

  #drawTargetBlip(ctx, bx, by, dotR, edge, color) {
    const px = this.#px;
    const r = Math.max(7 * px, dotR + 3 * px);
    ctx.save();
    ctx.translate(bx, by);
    ctx.strokeStyle = phColor(this, color);
    ctx.lineWidth = (edge ? 2.5 : 2) * px;

    ctx.beginPath();
    ctx.moveTo(0, -r);
    ctx.lineTo(r, 0);
    ctx.lineTo(0, r);
    ctx.lineTo(-r, 0);
    ctx.closePath();
    ctx.globalAlpha = edge ? 0.15 : 0.26;
    ctx.fillStyle = phColor(this, color);
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
      ctx.fillStyle = phColor(this, color);
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
    if (!this.canvas) return;
    const rect = this.canvas.getBoundingClientRect();
    const touch = e.touches ? e.touches[0] : e;
    const x = touch.clientX - rect.left;
    const y = touch.clientY - rect.top;
    const scaleX = this.canvas.width / rect.width;
    const scaleY = this.canvas.height / rect.height;
    const blip = this.#getBlipAt(x * scaleX, y * scaleY);
    if (blip && this.sendAction) {
      this.sendAction('set_target', { uuid: blip.uuid });
    }
  }

  #boundTap = (e) => this.#onPointerTap(e);
}

phDefine('ph-radar', PhRadar);
