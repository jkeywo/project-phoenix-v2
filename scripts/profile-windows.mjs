import fs from 'node:fs';
import { parse } from 'smol-toml';
const [profileFile, hardwareFile] = process.argv.slice(2);
const profile = parse(fs.readFileSync(profileFile, 'utf8').replace(/^\uFEFF/, ''));
const hardware = JSON.parse(fs.readFileSync(hardwareFile, 'utf8').replace(/^\uFEFF/, ''));
const windows = profile.display.map(display => {
  const monitor = hardware.displays.find(monitor => monitor.identity === display.id);
  if (!monitor) throw new Error(`Profile names a monitor absent from hardware provenance: ${display.id}`);
  return { monitor: display.id, role: display.role, width: monitor.width, height: monitor.height,
    scale: monitor.scalePercent / 100 };
});
if (!windows.some(window => window.role === 'viewscreen')) throw new Error('Profile must name a Viewscreen');
console.log(JSON.stringify(windows));
