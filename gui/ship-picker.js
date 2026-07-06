export function resolveShipSelection(ships) {
  if (!ships || ships.length === 0) {
    return { action: 'legacy-fallback' };
  }
  if (ships.length === 1) {
    return { action: 'auto-select', templatePath: ships[0].template_path };
  }
  return { action: 'show-picker', ships };
}
