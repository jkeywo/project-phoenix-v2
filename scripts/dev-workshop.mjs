// Direct project Workshop launcher retained for the retired editor/viewer commands.
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseWorkshopLaunch } from '../editor/workshop-launch.js';

export function workshopLaunchQuery(args, environmentQuery = '') {
  const allowed = new Set(['model', 'variant', 'entity', 'file', 'lighting', 'gizmos']);
  const seen = new Set();
  const requested = [...args];
  if (environmentQuery) {
    const environment = new URLSearchParams(environmentQuery);
    for (const [key, value] of environment) requested.push(`--${key}=${value}`);
  }
  for (const argument of requested) {
    if (argument === '--models') {
      if (seen.has('models')) throw new Error('Duplicate Workshop selector: --models');
      seen.add('models'); continue;
    }
    const match = /^--([a-z]+)=(.+)$/.exec(argument);
    if (!match || !allowed.has(match[1]) || seen.has(match[1])) {
      throw new Error(`Invalid Workshop selector: ${argument}`);
    }
    seen.add(match[1]);
  }
  if (seen.has('model') && seen.has('entity')) throw new Error('Choose one Workshop preview subject');
  const take = name => {
    const prefix = `--${name}=`; const found = args.find(value => value.startsWith(prefix));
    const direct = found?.slice(prefix.length) || null;
    if (direct) return direct;
    return environmentQuery ? new URLSearchParams(environmentQuery).get(name) : null;
  };
  const panel = take('model') || take('entity') ? 'model-preview' : args.includes('--models') ? 'models' : 'files';
  const launch = new URLSearchParams({ panel });
  for (const key of ['model', 'variant', 'entity', 'file', 'lighting', 'gizmos']) {
    const value = take(key); if (value) launch.set(key, value);
  }
  const parsed = parseWorkshopLaunch(launch);
  for (const key of seen) {
    if (key === 'models') continue;
    const accepted = key === 'model' ? parsed?.preview?.model : key === 'entity' ? parsed?.preview?.entity
      : key === 'variant' ? parsed?.preview?.variant : key === 'file' ? parsed?.file : parsed?.controls?.[key];
    const requested = key === 'gizmos' ? take(key) === '1' : take(key);
    if (accepted !== requested) throw new Error(`Invalid Workshop selector: --${key}`);
  }
  return launch.toString();
}
const run = (command, commandArgs) => {
  const result = spawnSync(command, commandArgs, { cwd: process.cwd(), stdio: 'inherit', shell: false });
  if (result.error) throw result.error;
  if (result.status) process.exit(result.status);
};
export function main(args = process.argv.slice(2)) {
  run('trunk', ['build']);
  run('cargo', ['build', '--features', 'host,ultralight', '--bin', 'phoenix-host']);
  const binary = path.join('target', 'debug', process.platform === 'win32' ? 'phoenix-host.exe' : 'phoenix-host');
  run(binary, ['--client-dir', 'dist', '--workshop-project', '.', '--workshop-open',
    workshopLaunchQuery(args, process.env.PHOENIX_WORKSHOP_OPEN || '')]);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
