import { buildNativeMatrix } from './profile-analysis.mjs';
console.log(JSON.stringify(buildNativeMatrix(Number(process.argv[2] || 3)), null, 2));
