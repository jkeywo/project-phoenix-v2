// Single-source helper for the "active console" tab selection.
//
// The inline `setActiveConsole(name)` function in `client.html` delegates
// the null/undefined/"" → next normalisation here so the contract is locked
// by a Vitest test instead of by inspection of the inline `<script>`.

// Pure: given the current active console and a new name (string, null,
// undefined, or empty string), returns what the inline `setActiveConsole`
// should do next.
//
// Returns:
//   { changed: bool, next: string|null }
//
// Where:
//   - `next` is the value to store in `activeConsole` (null when no console
//     is active — empty string and undefined both normalise to null).
//   - `changed` is true when `next !== current` — callers use this to skip
//     redundant work on no-change rerenders.
export function nextActiveConsole(current, name) {
  const next = name || null;
  const cur = current || null;
  return {
    changed: next !== cur,
    next,
  };
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.nextActiveConsole = nextActiveConsole;
}
