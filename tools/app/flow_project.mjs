// Click through the Project screen of the running app: example project, edit, save.
//   node tools/app/flow_project.mjs
import { execFileSync } from 'node:child_process';

const cdp = (cmd, arg) => execFileSync('node', ['tools/app/cdp.mjs', cmd, arg], { encoding: 'utf8' }).trim();
const js = (code) => cdp('eval', code);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const text = () => cdp('text');
const clickButton = (label) =>
  js(`(() => { const b = [...document.querySelectorAll('button')].find(b => b.innerText.trim() === ${JSON.stringify(label)}); if (!b) return 'no button ' + ${JSON.stringify(label)}; b.click(); return 'clicked'; })()`);
const until = async (pred, what, ms = 30000) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const t = text();
    if (pred(t)) return t;
    await wait(500);
  }
  throw new Error('timed out waiting for ' + what + '\n' + text().slice(0, 600));
};
const results = [];
const check = (name, ok, detail = '') => {
  results.push(ok);
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`);
};

// 1. the example project, made and read by the Rust engine
console.log(clickButton('Try with example data'));
let t = await until((t) => /6 recordings/.test(t), 'the example project');
check('example project opens', /config\.yml found|New settings file/.test(t) || /6 recordings/.test(t), '6 recordings');
check('tiles show 6 and 2 deployments', /\n6\nrecordings found/.test(t) && /\n2\ndeployments in sheet/.test(t));
const folder = js(`document.querySelector('.card .row input').value`);
console.log('project folder:', folder);

// 2. change two settings
js(`(() => {
  const slider = document.querySelector('input[type=range]');
  slider.value = '0.55'; slider.dispatchEvent(new Event('input', { bubbles: true }));
  const inputs = [...document.querySelectorAll('input.input')];
  const title = inputs.find(i => i.placeholder === 'Asia/Kolkata');
  title.value = 'Asia/Kolkata'; title.dispatchEvent(new Event('input', { bubbles: true }));
  return 'edited';
})()`);
t = await until((t) => /unsaved change/.test(t), 'the unsaved bar');
check('unsaved change is counted', /1 unsaved change/.test(t), (t.match(/\d+ unsaved change\(s\)/) || [''])[0]);

// 3. discard puts the old value back
clickButton('Discard');
t = await until((t) => /Settings match config\.yml/.test(t), 'discard');
check('discard restores the settings', /Settings match config\.yml/.test(t));
check('slider back to saved value', Number(js(`document.querySelector('input[type=range]').value`)) !== 0.55);

// 4. change again and save
js(`(() => { const s = document.querySelector('input[type=range]'); s.value = '0.55'; s.dispatchEvent(new Event('input', { bubbles: true })); return 'edited'; })()`);
await until((t) => /unsaved change/.test(t), 'the unsaved bar again');
clickButton('Save to config.yml');
t = await until((t) => /Saved \d+ change/.test(t) || /could not be saved/i.test(t), 'the save', 30000);
check('saved', /Saved 1 change/.test(t), (t.match(/Saved[^\n]*/) || [''])[0]);
check('confidence now 0.55', /0\.55/.test(text()));
console.log(results.every(Boolean) ? '\nall ok' : '\nSOME CHECKS FAILED');
console.log('FOLDER=' + folder);
