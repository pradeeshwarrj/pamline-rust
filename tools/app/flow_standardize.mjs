// Click through the Standardize screen of the running app on a made-up raw folder.
//   node tools/app/flow_standardize.mjs <scratch folder>
// Makes <scratch>/std_project with config + raw recordings, opens it in the app,
// checks the raw files (dry run), runs the standardization and verifies the files.
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
const until = async (pred, what, ms = 60000) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const t = text();
    if (pred(t)) return t;
    await wait(500);
  }
  throw new Error('timed out waiting for ' + what + '\n' + text().slice(0, 900));
};
const results = [];
const check = (name, ok, detail = '') => {
  results.push(ok);
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`);
};

// ---- a project with a raw folder
const root = path.join(scratch, 'std_project').replace(/\\/g, '/');
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(`${root}/config`, { recursive: true });
fs.mkdirSync(`${root}/data/raw/A`, { recursive: true });
fs.mkdirSync(`${root}/data/raw/B`, { recursive: true });
fs.mkdirSync(`${root}/metadata`, { recursive: true });
fs.writeFileSync(`${root}/config/config.yml`, 'paths:\n  project_root: "."\nfilenames:\n  timezone: "Asia/Kolkata"\nstandardization:\n  min_duration_s: 5\n');
fs.writeFileSync(`${root}/metadata/deployments.csv`, 'deployment_id,recorder,grid,habitat,deployer,latitude,longitude,deploy_date,deploy_time,retrieve_date,retrieve_time\n1,SW1,59,Forest,Asha,7,93,01-04-2026,06:00,10-04-2026,18:00\n');
function wav(file, seconds, rate, channels) {
  const n = Math.floor(seconds * rate);
  const data = Buffer.alloc(n * channels * 2);
  for (let i = 0; i < n; i++) for (let c = 0; c < channels; c++) data.writeInt16LE(Math.round(8000 * Math.sin((2 * Math.PI * 440 * i) / rate)), (i * channels + c) * 2);
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVEfmt ', 8); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20);
  h.writeUInt16LE(channels, 22); h.writeUInt32LE(rate, 24); h.writeUInt32LE(rate * channels * 2, 28); h.writeUInt16LE(channels * 2, 32);
  h.writeUInt16LE(16, 34); h.write('data', 36); h.writeUInt32LE(data.length, 40);
  fs.writeFileSync(file, Buffer.concat([h, data]));
}
wav(`${root}/data/raw/A/G0059_20260405_060000.wav`, 10, 8000, 1);
wav(`${root}/data/raw/A/G0059_20260405_060010.wav`, 10, 44100, 2);
wav(`${root}/data/raw/B/G0059_20260405_060020.wav`, 10, 8000, 1);
wav(`${root}/data/raw/B/short_memo.wav`, 2, 8000, 1);
fs.writeFileSync(`${root}/data/raw/B/notes.txt`, 'field notes');
fs.writeFileSync(`${root}/data/raw/B/bad.wav`, 'not audio at all');

// ---- open it in the app, go to Standardize
js(`localStorage.setItem('pamflow-project-folder', ${JSON.stringify(root)}); location.reload()`);
await wait(4000);
click('Standardize', 'button.step');
let t = await until((t) => /Step 2 · Standardize/.test(t) && /raw/i.test(t), 'the Standardize screen');
t = await until((t) => /\b6 files?\b|6 file/.test(t) || /Check the raw files/i.test(t), 'the raw file count');
check('screen opens with the raw folder counted', /Step 2 · Standardize/.test(t), (t.match(/[^\n]*6 files?[^\n]*/) || [''])[0]);
check('sidebar marks Standardize', /2 Standardize/.test(js(`document.querySelector('button.step.active').innerText.replace(/\\s+/g,' ').trim()`)));

// ---- check (dry run)
console.log(click('Check raw files'));
t = await until((t) => /Check again/.test(t) && /Standardize 3 recordings/.test(t), 'the plan');
console.log(t.split('\n').filter((l) => l.trim()).slice(-34).join(' | ').slice(0, 900));
check('plan shows 3 recordings', /\n3\nrecordings/.test(t) || /3\s*\n\s*recordings/i.test(t));

// ---- run
const startLabel = js(`[...document.querySelectorAll('button.btn.primary.lg')].map(b => b.innerText.trim()).join('|')`);
console.log('buttons:', startLabel);
console.log(click('Standardize 3'));
t = await until((t) => /Finished/.test(t) && /Last run/.test(t), 'the run to finish', 120000);
await wait(1500);
t = text();
console.log(t.split('\n').filter((l) => l.trim()).slice(-30).join(' | ').slice(0, 900));
const out = `${root}/data/standardized`;
const files = fs.existsSync(out) ? fs.readdirSync(out, { recursive: true }).map(String) : [];
console.log('written:', files.join(', '));
check('3 recordings written as flac', files.filter((f) => f.endsWith('.flac')).length === 3);
check('log file written', files.some((f) => f.endsWith('standardization_log.csv')));
const iso = `${root}/data/isolated`;
const isoFiles = fs.existsSync(iso) ? fs.readdirSync(iso, { recursive: true }).map(String) : [];
console.log('isolated:', isoFiles.join(', '));
check('memo, text and unreadable file set aside', isoFiles.length >= 3);
console.log(results.every(Boolean) ? '\nall ok' : '\nSOME CHECKS FAILED');
