// Click through the Scan screen of the running app.
//   node tools/app/flow_scan.mjs
// 1. open Scan on the remembered project: shows the last scan (nothing is run)
// 2. make an example project, run a scan on it, check the results
import { execFileSync } from 'node:child_process';

const cdp = (cmd, arg) => execFileSync('node', ['tools/app/cdp.mjs', cmd, arg], { encoding: 'utf8' }).trim();
const js = (code) => cdp('eval', code);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const text = () => cdp('text');
const click = (selector, label) =>
  js(`(() => { const b = [...document.querySelectorAll(${JSON.stringify(selector)})].find(b => b.innerText.trim() === ${JSON.stringify(label)} || b.innerText.trim().endsWith(${JSON.stringify(label)})); if (!b) return 'no ${label}'; b.click(); return 'clicked'; })()`);
const until = async (pred, what, ms = 40000) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const t = text();
    if (pred(t)) return t;
    await wait(500);
  }
  throw new Error('timed out waiting for ' + what + '\n' + text().slice(0, 700));
};
const results = [];
const check = (name, ok, detail = '') => {
  results.push(ok);
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`);
};

js("localStorage.setItem('pamflow-project-folder', 'E:/Pradeeshwar/pamflow_pilot'); location.reload()");
await wait(4000);

// 1. the last scan of the remembered project
console.log(click('button.step', 'Scan'));
let t = await until((t) => /Step 3 · Scan/.test(t) && !/Reading the project/.test(t), 'the Scan screen');
t = await until((t) => /recordings in|files read|Files|Re-scan|Scan the recordings|Scan again/i.test(t), 'the scan summary or run button');
console.log(t.split('\n').slice(0, 24).join(' | '));
check('Scan screen opens', /Step 3 · Scan/.test(t));
const sidebarActive = js(`document.querySelector('button.step.active').innerText.replace(/\\s+/g,' ').trim()`);
check('sidebar marks Scan', /3 Scan/.test(sidebarActive), sidebarActive);

// 2. a new example project and a scan
click('button.step', 'Project');
await until((t) => /Step 1 · Project/.test(t), 'back on Project');
click('button', 'Try with example data');
await until((t) => /6 recordings/.test(t), 'the example project');
click('button.step', 'Scan');
t = await until((t) => /Step 3 · Scan/.test(t), 'Scan again');
await wait(1500);
const label = js(`[...document.querySelectorAll('button.btn.primary.lg')].map(b => b.innerText.trim()).join('|')`);
console.log('run button:', label);
click('button.btn.primary.lg', label.split('|')[0]);
t = await until((t) => /Scan again|Scan the recordings again|Re-scan/i.test(t) && /\n6\n/.test(t), 'the finished scan', 60000);
console.log(t.split('\n').slice(4, 60).join(' | '));
check('6 files and 2 grids', /\n6\nrecordings|\n6\nfiles|6\n/.test(t) && /\n2\n/.test(t));
check('checks list shown', /none/.test(t));
const folder = js(`localStorage.getItem('pamflow-project-folder')`);
console.log('FOLDER=' + folder);
console.log(results.every(Boolean) ? '\nall ok' : '\nSOME CHECKS FAILED');
