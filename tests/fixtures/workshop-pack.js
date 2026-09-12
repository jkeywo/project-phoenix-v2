import { createStoreZip } from '../../editor/mod-pack-export.js';

export const WORKSHOP_WORLD = 'assets/worlds/workshop.toml';
export const WORKSHOP_MANIFEST = '# Keep this manifest note\r\n[pack]\r\nformat = 1\r\nid = "workshop-test"\r\nversion = "1.0.0"\r\nname = "Workshop test" # identity\r\ncustom_note = "retain extension"\r\n[pack.requires]\r\ncontent_id = "phoenix-base"\r\ncontent_epoch = 1\r\n[[scenario]]\r\nid = "workshop-test"\r\nworld = "assets/worlds/workshop.toml"\r\n';
export const WORKSHOP_WORLD_TEXT = '# Keep world comments\r\n[global]\r\n[anchors]\r\n';
export function workshopPack() {
  return createStoreZip([
    { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT },
  ]);
}
