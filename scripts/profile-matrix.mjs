import { buildNativeMatrix } from './profile-analysis.mjs';
const [repetitions = '3', mode = 'legacy', experiment = ''] = process.argv.slice(2);
if (!['legacy', 'three'].includes(mode)) throw new Error('Expected legacy or three matrix mode');
console.log(JSON.stringify(buildNativeMatrix(Number(repetitions), {
  threeStations: mode === 'three', experiment,
}), null, 2));
