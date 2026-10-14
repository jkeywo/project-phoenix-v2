#!/usr/bin/env node
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve, dirname, relative, sep } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const base = resolve(root, 'docs/gameplay-redesign/implementation');
const ledger = JSON.parse(readFileSync(resolve(base, 'migration-ledger.json'), 'utf8'));
const review = readFileSync(resolve(root, 'docs/gameplay-redesign/system-review.md'), 'utf8');
const headings = [...review.matchAll(/^## ([GS]\d{3}) — (.+)$/gm)];
const expected = new Map(headings.map(m => [m[1], m[2].trim()]));
function requireCondition(condition, message) { if (!condition) throw new Error(message); }
requireCondition(ledger.schema_version === 1, 'Unsupported migration ledger version');
requireCondition(expected.size === 216 && headings.length === 216, 'Expected 216 unique catalogue entries');
requireCondition(ledger.increments.length === 25, 'Expected increments 0–24');
const statuses = new Set(['pending', 'in_progress', 'complete', 'blocked']);
if (ledger.tracker) requireCondition(/^https:\/\/github\.com\/jkeywo\/project-phoenix-v2\/issues\/\d+$/.test(ledger.tracker.url), 'Invalid execution tracker');
for (const [i, step] of ledger.increments.entries()) {
  requireCondition(step.id === i && statuses.has(step.status), `Invalid increment ${i}`);
  requireCondition(JSON.stringify(step.depends_on) === JSON.stringify(i ? [i - 1] : []), `Serial dependency changed at ${i}`);
  requireCondition(step.title && step.completion && Array.isArray(step.evidence), `Missing contract for increment ${i}`);
  if (step.status === 'complete') requireCondition(step.evidence.length > 0, `Completed increment ${i} has no evidence`);
  for (const evidence of step.evidence) {
    const bytes = readFileSync(resolve(root,evidence),'utf8');
    if (step.status==='complete' && evidence.endsWith('.json')) requireCondition(JSON.parse(bytes).status==='passed', `Incomplete evidence ${evidence}`);
  }
  for (const sub of step.subincrements || []) requireCondition(statuses.has(sub.status), `Invalid sub-increment ${sub.id}`);
  if (step.status === 'complete') requireCondition((step.subincrements||[]).every(s=>s.status==='complete'), `Incomplete sub-increment in ${i}`);
}
requireCondition(Number.isInteger(ledger.current_increment) && ledger.current_increment >= 0 && ledger.current_increment < 25, 'Invalid current increment');
for (const authorization of ledger.advance_authorizations || []) {
  requireCondition(Number.isInteger(authorization.from_increment) && authorization.to_increment === authorization.from_increment + 1, 'Invalid advance authorization');
  requireCondition(authorization.user_instruction && authorization.date && authorization.pending_acceptance, 'Advance authorization must record the instruction and outstanding acceptance');
  requireCondition(existsSync(resolve(root, authorization.evidence || 'missing')), 'Advance authorization needs acceptance evidence');
}
for (let i = 0; i < ledger.current_increment; i++) {
  const authorized = (ledger.advance_authorizations || []).some(a => a.from_increment === i && a.to_increment === i + 1);
  requireCondition(ledger.increments[i].status === 'complete' || authorized, `Increment ${i} is incomplete`);
}
const seen = new Set();
for (const feature of ledger.features) {
  requireCondition(expected.get(feature.id) === feature.title && !seen.has(feature.id), `Missing, duplicate or renamed feature ${feature.id}`);
  seen.add(feature.id);
  requireCondition(['migration', 'retained', 'excluded'].includes(feature.disposition), `Invalid disposition for ${feature.id}`);
  requireCondition(feature.disposition === 'excluded' ? feature.increments.length === 0 : feature.increments.length > 0, `Missing implementation home for ${feature.id}`);
  requireCondition(new Set(feature.increments).size === feature.increments.length, `Duplicate milestone for ${feature.id}`);
  requireCondition(feature.increments.every(i => Number.isInteger(i) && i >= 0 && i < 25), `Invalid milestone for ${feature.id}`);
}
requireCondition(seen.size === expected.size, 'Catalogue coverage is incomplete');
for (const step of ledger.increments) requireCondition(ledger.features.some(f=>f.increments.includes(step.id)), `Increment ${step.id} has no catalogue consumer`);
for (const item of ledger.superseded_future_choices) requireCondition(readFileSync(resolve(root, item.file), 'utf8').includes(item.entity), `Missing PASM source ${item.entity}`);
requireCondition(ledger.constraints.message_capacity_tracking === false, 'Message capacity is excluded');

const esc = s => String(s).replace(/[&<>"']/g, c => ({'&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;'}[c]));
const steps = ledger.increments.map(step => `<tr id="increment-${step.id}"><td>${step.id}</td><td><strong>${esc(step.title)}</strong>${(step.subincrements||[]).map(s=>`<p>${esc(s.id)} · ${esc(s.title)} · ${esc(s.status)}</p>`).join('')}</td><td>${esc(step.status)}</td><td>${esc(step.completion)}</td><td>${step.evidence.map(p=>`<a href="${esc(relative(base,resolve(root,p)).split(sep).join('/'))}">${esc(p.split('/').at(-1))}</a>`).join('<br>')||'Pending'}</td></tr>`).join('\n');
const features = ledger.features.map(f => `<tr data-catalogue="${f.id[0]}" data-increments="${f.increments.join(',')}"><td>${f.id}</td><td>${esc(f.title)}</td><td>${esc(f.disposition)}</td><td>${f.increments.map(i => `<a href="#increment-${i}">${i}</a>`).join(', ') || '—'}</td><td>${esc(f.note)}</td></tr>`).join('\n');
const conflicts = ledger.superseded_future_choices.map(c => `<tr><td><code>${esc(c.entity)}</code></td><td>${esc(c.old)}</td><td>${esc(c.new)}</td><td>${c.replacement_increment}</td></tr>`).join('\n');
const baselinePath = resolve(base,'baseline.json');
const baseline = existsSync(baselinePath) ? JSON.parse(readFileSync(baselinePath,'utf8')) : null;
const baselineHtml = baseline ? `<section id="baseline"><h2>Recorded baseline</h2><p>Revision <code>${esc(baseline.revision)}</code>; ${esc(baseline.profile)} profile; Alliance Cruiser; seed 42; 60 simulated seconds per world. Headless same-machine comparisons exclude the first 300 harness updates. These are not rendering or release-budget acceptance measurements.</p><p>Both worlds reproduce seeded reports and replay checkpoints. Cruiser matrix: ${baseline.cruiser_balance.victories}/${baseline.cruiser_balance.runs} victories; ${baseline.cruiser_balance.thresholds.filter(t=>t.pass).length} thresholds passed.</p><div class="table-wrap"><table><thead><tr><th>Capture</th><th>Updates</th><th>p50 update ms</th><th>p95 update ms</th></tr></thead><tbody>${baseline.performance.map(r=>`<tr><td>${esc(r.name)}</td><td>${r.metrics['sim.tick'].count}</td><td>${r.metrics['sim.tick'].p50.toFixed(3)}</td><td>${r.metrics['sim.tick'].p95.toFixed(3)}</td></tr>`).join('')}</tbody></table></div><p>Evidence: <a href="baseline.json">native baseline</a>, <a href="validation.json">validation receipt</a>, <a href="baseline.md">interpretation and limitations</a>.</p></section>` : '';
const workflowHtml = ledger.validation_workflow ? `<p class="note">${esc(ledger.validation_workflow.description)}</p>` : '';
const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Phoenix · Implementation roadmap</title>
<style>:root{color-scheme:light}*{box-sizing:border-box}body{margin:0;background:#f3f5f7;color:#172b3b;font:16px/1.6 system-ui,sans-serif}main{max-width:1250px;margin:auto;padding:36px 24px 70px}header{border-top:5px solid #076d68;padding-top:20px}h1{font-size:clamp(30px,4vw,44px);line-height:1.2}h2{font-size:25px}section{background:white;border:1px solid #ccd7df;border-radius:10px;padding:25px;margin-top:24px}a{color:#076d68}nav{display:flex;flex-wrap:wrap;gap:20px}.badge{display:inline-block;border-radius:16px;background:#e5f4ed;padding:4px 12px}.note{padding:12px 16px;border-left:4px solid #076d68;background:#f0f8f6}.table-wrap{overflow:auto}table{width:100%;border-collapse:collapse;font-size:14px}th,td{text-align:left;vertical-align:top;padding:10px 12px;border-bottom:1px solid #ccd7df}th{background:#eef3f6}svg{width:100%;min-width:700px;height:auto}svg text{font-family:system-ui,sans-serif;fill:#172b3b;font-size:15px}.filters{display:flex;gap:16px;flex-wrap:wrap}label{display:flex;flex-direction:column;gap:4px}input,select{font:inherit;padding:8px;border:1px solid #71899a;border-radius:4px}a:focus-visible,input:focus-visible,select:focus-visible{outline:3px solid #a25105;outline-offset:3px}[hidden]{display:none!important}footer{font-size:14px;color:#526777;margin-top:24px}@media(max-width:700px){main{padding:20px 12px}section{padding:18px 14px}}@media print{body{background:white}.filters,nav{display:none}tr{break-inside:avoid}svg{min-width:0}}</style></head><body><main>
<header><p>Project Phoenix / Approved implementation sequence / ${esc(ledger.approved_date)}</p><h1>One playable increment at a time</h1><span class="badge">Current increment: ${ledger.current_increment}</span>${ledger.tracker ? '<p>Execution tracker: <a href="' + esc(ledger.tracker.url) + '">PRD #' + esc(ledger.tracker.issue) + '</a></p>' : ''}<p>The serial roadmap preserves shipped content, existing networking and individual System AI. Older saves and external packs use explicit compatibility checks or conversion. Each capability includes human controls, AI, Workshop support and continuation state.</p><nav><a href="#sequence">Sequence</a><a href="#conflicts">Design reconciliation</a><a href="#coverage">216-feature ledger</a><a href="README.md">Evidence and working rules</a><a href="../architecture/feature-reconciliation.html">Mechanism design</a></nav></header>
<section id="sequence"><h2>Serial implementation sequence</h2><div class="table-wrap"><svg viewBox="0 0 1050 150" role="img" aria-labelledby="order-title"><title id="order-title">Initial increment commit, isolated validation, then dependency-order integration</title><defs><marker id="order-arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0 0L8 4L0 8Z" fill="#456378"/></marker></defs>${['Baseline','Initial increment commit','Isolated validation','Integrate fixes'].map((t,i)=>`<rect x="${20+i*265}" y="35" width="220" height="70" rx="7" fill="#e5f4ed" stroke="#70899a"/><text x="${130+i*265}" y="77" text-anchor="middle">${t}</text>${i<3?`<path d="M${240+i*265} 70H${285+i*265}" stroke="#456378" stroke-width="2" marker-end="url(#order-arrow)"/>`:''}`).join('')}</svg></div>${workflowHtml}<p>Structural migrations preserve observable behaviour first. Gameplay changes then receive their own balance/activity evidence. Milestones can contain multiple small changes; no competing authoritative implementation is retained.</p><div class="table-wrap"><table><thead><tr><th>#</th><th>Increment</th><th>Status</th><th>Playable exit</th><th>Evidence</th></tr></thead><tbody>${steps}</tbody></table></div></section>
${baselineHtml}
<section id="conflicts"><h2>Scheduled replacement of earlier future choices</h2><p>These selected redesign choices supersede earlier plans. Current runtime descriptions remain valid until their replacement increment lands. No orbital, scanning or power runtime is changed by the baseline increment.</p><div class="table-wrap"><table><thead><tr><th>PASM entity</th><th>Earlier plan</th><th>Selected replacement</th><th>Increment</th></tr></thead><tbody>${conflicts}</tbody></table></div><p class="note">Lossy used-coolant recycling remains provisional. Separate first-person and repair/transcription puzzle layers are excluded. Message-capacity accounting is excluded.</p></section>
<section id="coverage"><h2>Complete catalogue ledger</h2><p>Retained capabilities and cross-cutting experience requirements apply throughout affected work. A mapping is an implementation home, not evidence that a feature has shipped.</p><div class="filters"><label>Find feature<input id="search" type="search" placeholder="ID, name or note"></label><label>Catalogue<select id="catalogue"><option value="all">Both</option><option value="G">Game systems</option><option value="S">Ship systems</option></select></label><label>Increment<select id="increment"><option value="all">All</option>${ledger.increments.map(s=>`<option value="${s.id}">${s.id} · ${esc(s.title)}</option>`).join('')}</select></label></div><p id="count" role="status" aria-live="polite">216 entries</p><div class="table-wrap"><table id="features"><thead><tr><th>ID</th><th>Feature</th><th>Disposition</th><th>Increments</th><th>Constraint</th></tr></thead><tbody>${features}</tbody></table></div></section><footer>Generated from <a href="migration-ledger.json">migration-ledger.json</a> by scripts/redesign-ledger.mjs. Regenerate with --write and check with --check. Evidence: <a href="README.md">baseline and milestone receipts</a>.</footer></main>
<script>(()=>{const rows=[...document.querySelectorAll('#features tbody tr')],search=document.getElementById('search'),catalogue=document.getElementById('catalogue'),increment=document.getElementById('increment');function filter(){const terms=search.value.toLowerCase().trim().split(/\\s+/).filter(Boolean);let n=0;for(const row of rows){const ok=(catalogue.value==='all'||row.dataset.catalogue===catalogue.value)&&(increment.value==='all'||row.dataset.increments.split(',').includes(increment.value))&&terms.every(t=>row.textContent.toLowerCase().includes(t));row.hidden=!ok;if(ok)n++}document.getElementById('count').textContent=n+' of '+rows.length+' entries shown'}search.addEventListener('input',filter);catalogue.addEventListener('change',filter);increment.addEventListener('change',filter)})();</script></body></html>
`;
const mode = process.argv[2];
requireCondition(process.argv.length === 3 && ['--check','--write'].includes(mode), 'Usage: node scripts/redesign-ledger.mjs --check|--write');
const output = resolve(base, 'index.html');
if (mode === '--write') writeFileSync(output, html);
else requireCondition(readFileSync(output, 'utf8') === html, 'Roadmap HTML is stale; run node scripts/redesign-ledger.mjs --write');
console.log(`Redesign ledger: ${seen.size} catalogue entries, 25 serial increments; ${mode === '--write' ? 'report generated' : 'coverage and report current'}.`);
