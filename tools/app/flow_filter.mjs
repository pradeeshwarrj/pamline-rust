// Click through the Filter screen of the running app: scan first, run, Stop, Continue.
//   node tools/app/flow_filter.mjs <scratch folder>
// Makes <scratch>/filter_project with 60 one-minute recordings (pink noise), opens it in the
// app, starts filtering (the engine scans first), stops it part way, continues, and checks
// that every file was filtered once.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const scratch = process.argv[2];
const cdp = (cmd, arg) => execFileSync('node', ['tools/app/cdp.mjs', cmd, arg], { encoding: 'utf8' }).trim();
const js = (code) => cdp('eval', code);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const text = () => cdp('text');
const click = (label, selector = 'button') =>
  js(`(() => { const b = [...document.querySelectorAll(${JSON.stringify(selector)})].find(b => b.innerText.trim().includes(${JSON.stringify(label)})); if (!b) return 'no button ' + ${JSON.stringify(label)}; if (b.disabled) return 'disabled ' + ${JSON.stringify(label)}; b.click(); return 'clicked'; })()`);
const until = async (pred, what, ms = 90000, every = 300) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const t = text();
    if (pred(t)) return t;
    await wait(every);
  }
  throw new Error('timed out waiting for ' + what + '\n' + text().slice(0, 900));
};
const results = [];
const check = (name, ok, detail = '') => {
  results.push(ok);
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`);
};
const N = 60;

// ---- a project with N recordings
const root = path.join(scratch, 'filter_project').replace(/\\/g, '/');
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(`${root}/config`, { recursive: true });
fs.mkdirSync(`${root}/data/standardized/G0059`, { recursive: true });
fs.mkdirSync(`${root}/metadata`, { recursive: true });
fs.writeFileSync(`${root}/config/config.yml`, 'paths:\n  project_root: "."\nfilenames:\n  timezone: "Asia/Kolkata"\nfilter:\n  workers: 2\n');
fs.writeFileSync(`${root}/metadata/deployments.csv`, 'deployment_id,recorder,grid,habitat,deployer,latitude,longitude,deploy_date,deploy_time,retrieve_date,retrieve_time\n1,SW1,59,Forest,Asha,7,93,01-04-2026,00:00,30-04-2026,23:00\n');
const first = `${root}/data/standardized/G0059/G0059_20260405_060000.wav`;
execFileSync('ffmpeg', ['-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'anoisesrc=d=60:c=pink:r=48000:a=0.3', '-ac', '1', '-c:a', 'pcm_s16le', first]);
const pad = (n) => String(n).padStart(2, '0');
for (let i = 1; i < N; i++) {
  const t = new Date(Date.UTC(2026, 3, 5, 6, 0, 0) + i * 70000);
  const stamp = `${t.getUTCFullYear()}${pad(t.getUTCMonth() + 1)}${pad(t.getUTCDate())}_${pad(t.getUTCHours())}${pad(t.getUTCMinutes())}${pad(t.getUTCSeconds())}`;
  fs.copyFileSync(first, `${root}/data/standardized/G0059/G0059_${stamp}.wav`);
}

// ---- open it, go to Filter
js(`localStorage.setItem('pamflow-project-folder', ${JSON.stringify(root)}); location.reload()`);
await wait(4000);
click('Filter', 'button.step');
let t = await until((t) => /Step 4 · Filter/.test(t) && /Start filtering|High-pass|filter/i.test(t), 'the Filter screen');
console.log(t.split('\n').filter((l) => l.trim()).slice(7, 30).join(' | ').slice(0, 600));
check('screen opens', /Step 4 · Filter/.test(t));
check('sidebar marks Filter', /4 Filter/.test(js(`document.querySelector('button.step.active').innerText.replace(/\\s+/g,' ').trim()`)));
const startButton = js(`[...document.querySelectorAll('button.btn.primary.lg')].map(b => b.innerText.trim()).join('|')`);
console.log('start button:', startButton);

// ---- start; Stop part way
console.log(click(startButton.replace('▶ ', '')));
t = await until((t) => /\n([1-9]\d*)\nfiles done/.test(t), 'some files to be done', 60000, 150);
check('progress while running', /Stop/.test(t));
console.log(click('Stop'));
t = await until((t) => /Stopped|stopped/.test(t), 'the run to stop', 30000);
check('stopped', /stopped/i.test(t));
const doneAtStop = fs.readdirSync(`${root}/data/filtered/G0059`).filter((f) => f.endsWith('.wav')).length;
console.log(`filtered at stop: ${doneAtStop} of ${N}`);
check('stopped part way', doneAtStop > 0 && doneAtStop < N, `${doneAtStop}`);

// ---- continue
await wait(1500);
const label2 = js(`[...document.querySelectorAll('button.btn.primary.lg')].map(b => b.innerText.trim()).join('|')`);
console.log('button after stop:', label2);
check('button offers to continue', /Continue|continue/.test(label2));
console.log(click(label2.split('|')[0].replace('▶ ', '')));
t = await until((t) => /Finished/.test(t), 'the run to finish', 120000);
await wait(1000);
const files = fs.readdirSync(`${root}/data/filtered/G0059`).filter((f) => f.endsWith('.wav'));
const part = fs.readdirSync(`${root}/data/filtered/G0059`).filter((f) => f.endsWith('.part'));
check(`all ${N} files filtered`, files.length === N, `${files.length}`);
check('no half-written files left', part.length === 0);
const done = fs.readFileSync(`${root}/metadata/02_filter.done`, 'utf8').split(/\r?\n/).filter(Boolean);
check('each file in the checkpoint once', new Set(done).size === N && done.length === N, `${done.length} lines`);
console.log(text().split('\n').filter((l) => l.trim()).slice(-14).join(' | ').slice(0, 500));
console.log(results.every(Boolean) ? '\nall ok' : '\nSOME CHECKS FAILED');
