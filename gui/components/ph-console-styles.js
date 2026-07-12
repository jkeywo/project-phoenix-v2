// gui/components/ph-console-styles.js — shared control styling for ph-* console
// components.
//
// The destroyer/cruiser consoles compose small Shadow-DOM web components. Shadow
// DOM blocks console.css's class rules (.btn, .chip …) from reaching inside a
// component, but CSS *custom properties* defined on :root DO inherit through the
// shadow boundary. So this module carries the shared control chrome — the
// chamfered navy LED-pill buttons and compact steppers used across the old
// full-screen consoles — as a constructable stylesheet that every component
// adopts, while all colours resolve from the console.css :root tokens
// (var(--loaded), var(--ink-dim) …). Retinting the whole fleet is therefore a
// console.css change; only the button *geometry* lives here.
//
// The values below are the console.css `.btn` / `.chip` design (chamfer
// clip-path, layered gradients, radial LED dots) scaled down to fit the packed
// multi-control columns of the per-hull consoles.

const CSS = `
:host { --btn-h: 1.9rem; --btn-cham: 0.34rem; }

/* ── LED-pill button (compact console.css .btn) ─────────────────── */
.btn {
  position: relative;
  height: var(--btn-h);
  padding: 0 0.65rem 0 1.6rem;
  display: inline-flex; align-items: center;
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 600;
  font-size: 0.62rem;
  letter-spacing: 0.18em;
  text-transform: uppercase;
  color: var(--ink-dim);
  background: linear-gradient(180deg, #5e6f96 0%, #3a4674 50%, #1a2148 100%);
  clip-path: polygon(
    var(--btn-cham) 0, calc(100% - var(--btn-cham)) 0, 100% var(--btn-cham),
    100% calc(100% - var(--btn-cham)), calc(100% - var(--btn-cham)) 100%,
    var(--btn-cham) 100%, 0 calc(100% - var(--btn-cham)), 0 var(--btn-cham)
  );
  border: none;
  cursor: pointer;
  user-select: none;
  touch-action: manipulation;
}
.btn > .btn-bg {
  position: absolute; inset: 1.5px;
  --btn-cham: 0.3rem;
  background: linear-gradient(180deg, #1a2148 0%, #0e1432 50%, #070b22 100%);
  clip-path: polygon(
    var(--btn-cham) 0, calc(100% - var(--btn-cham)) 0, 100% var(--btn-cham),
    100% calc(100% - var(--btn-cham)), calc(100% - var(--btn-cham)) 100%,
    var(--btn-cham) 100%, 0 calc(100% - var(--btn-cham)), 0 var(--btn-cham)
  );
  z-index: 0;
}
.btn > .led,
.btn > .label { position: relative; z-index: 1; }
.btn .led {
  position: absolute;
  left: 0.5rem; top: 50%;
  transform: translateY(-50%);
  width: 0.6rem; height: 0.6rem;
  border-radius: 50%;
  background: radial-gradient(circle at 35% 30%, #34406a 0%, #0a1028 70%);
  box-shadow: inset 0 0 3px rgba(0,0,0,0.6);
}
.btn .led.on    { background: radial-gradient(circle at 35% 30%, #aef0c0 0%, var(--loaded) 35%, var(--loaded-dim) 80%); box-shadow: 0 0 8px var(--loaded), inset 0 0 2px rgba(255,255,255,0.5); }
.btn .led.fire  { background: radial-gradient(circle at 35% 30%, #ffb89a 0%, var(--fire) 35%, var(--fire-dim) 80%); box-shadow: 0 0 8px var(--fire), inset 0 0 2px rgba(255,255,255,0.5); }
.btn .led.amber { background: radial-gradient(circle at 35% 30%, #ffd896 0%, var(--reloading) 35%, var(--reloading-dim) 80%); box-shadow: 0 0 7px var(--reloading), inset 0 0 2px rgba(255,255,255,0.5); }
.btn .led.cyan  { background: radial-gradient(circle at 35% 30%, #d0f0ff 0%, var(--cyan) 35%, var(--cyan-dim) 80%); box-shadow: 0 0 7px var(--cyan), inset 0 0 2px rgba(255,255,255,0.5); }

/* Variant colours — same gradients as console.css .btn.armed/.danger/… */
.btn.armed          { color: var(--loaded); background: linear-gradient(180deg, #4ec870 0%, #1e5028 60%, #0e2818 100%); }
.btn.armed > .btn-bg { background: linear-gradient(180deg, #0e3422 0%, #082014 50%, #04120a 100%); }
.btn.danger         { color: var(--fire-bright); background: linear-gradient(180deg, #ff5a3a 0%, #6c1a14 60%, #2c0a08 100%); }
.btn.danger > .btn-bg { background: linear-gradient(180deg, #3a0e0e 0%, #200808 50%, #100404 100%); }
.btn.tactical       { color: var(--tactical); background: linear-gradient(180deg, #f08438 0%, #8c4818 60%, #3a1e0c 100%); }
.btn.tactical > .btn-bg { background: linear-gradient(180deg, #2a1408 0%, #180a04 50%, #0a0402 100%); }
.btn.disabled, .btn:disabled { color: var(--ink-faint); background: linear-gradient(180deg, #2a3050 0%, #14182c 100%); cursor: default; }
.btn.disabled > .btn-bg, .btn:disabled > .btn-bg { background: linear-gradient(180deg, #0a0f28 0%, #050918 100%); }
.btn:disabled .led:not(.keep) { background: radial-gradient(circle at 35% 30%, #34406a 0%, #0a1028 70%); box-shadow: inset 0 0 3px rgba(0,0,0,0.6); }
.btn:not(:disabled):hover { filter: brightness(1.15); }

/* No-LED pill (label centred) for toggles/short actions */
.btn.plain { padding: 0 0.7rem; justify-content: center; }

/* ── Mini chamfered stepper (compact console.css .chip) ─────────── */
.mini-btn {
  --btn-cham: 0.22rem;
  position: relative;
  width: 1.4rem; height: 1.4rem; flex-shrink: 0; padding: 0;
  display: inline-flex; align-items: center; justify-content: center;
  font-family: 'Chakra Petch', sans-serif; font-weight: 700; font-size: 0.9rem;
  color: var(--ink-dim);
  background: linear-gradient(180deg, #5e6f96 0%, #3a4674 50%, #1a2148 100%);
  clip-path: polygon(
    var(--btn-cham) 0, calc(100% - var(--btn-cham)) 0, 100% var(--btn-cham),
    100% calc(100% - var(--btn-cham)), calc(100% - var(--btn-cham)) 100%,
    var(--btn-cham) 100%, 0 calc(100% - var(--btn-cham)), 0 var(--btn-cham)
  );
  border: none; cursor: pointer; touch-action: manipulation;
}
.mini-btn > .mini-bg {
  position: absolute; inset: 1.5px; --btn-cham: 0.18rem;
  background: linear-gradient(180deg, #0e1432 0%, #060b1f 100%);
  clip-path: polygon(
    var(--btn-cham) 0, calc(100% - var(--btn-cham)) 0, 100% var(--btn-cham),
    100% calc(100% - var(--btn-cham)), calc(100% - var(--btn-cham)) 100%,
    var(--btn-cham) 100%, 0 calc(100% - var(--btn-cham)), 0 var(--btn-cham)
  );
  z-index: 0;
}
.mini-btn > .lbl { position: relative; z-index: 1; line-height: 1; }
.mini-btn:not(:disabled):hover { filter: brightness(1.2); }
.mini-btn:disabled { opacity: 0.35; cursor: default; }
`;

let sheet;
try {
  sheet = new CSSStyleSheet();
  sheet.replaceSync(CSS);
} catch (e) {
  // Older engines without constructable stylesheets: callers fall back to a
  // <style> element (see phAdoptConsoleStyles).
  sheet = null;
}

export const phConsoleStyles = sheet;
export const phConsoleStylesText = CSS;

// Adopt the shared control styles into a component's shadow root, with a
// <style>-element fallback for engines lacking constructable stylesheets.
export function phAdoptConsoleStyles(shadowRoot) {
  if (phConsoleStyles && 'adoptedStyleSheets' in shadowRoot) {
    shadowRoot.adoptedStyleSheets = [...shadowRoot.adoptedStyleSheets, phConsoleStyles];
  } else {
    const style = document.createElement('style');
    style.textContent = phConsoleStylesText;
    shadowRoot.appendChild(style);
  }
}
