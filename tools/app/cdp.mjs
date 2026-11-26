// Drive the running pamflow app (WebView2) over the Chrome DevTools protocol.
//
//   node tools/app/cdp.mjs eval  "<javascript>"        run JS in the page, print the result
//   node tools/app/cdp.mjs shot  <file.png>            screenshot the page
//   node tools/app/cdp.mjs text                        the visible text of the page
//
// The app must have been started with
//   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 cargo tauri dev
import fs from 'node:fs';

const [, , cmd, arg] = process.argv;
const list = await (await fetch('http://127.0.0.1:9222/json')).json();
const page = list.find((t) => t.type === 'page' && !t.url.startsWith('devtools'));
if (!page) {
  console.error('no page found; is the app running with the debugging port?');
  process.exit(2);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) pending.get(d.id)(d);
};
const send = (method, params = {}) =>
  new Promise((res) => {
    const n = ++id;
    pending.set(n, res);
    ws.send(JSON.stringify({ id: n, method, params }));
  });

async function evaluate(expression) {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) return 'ERROR: ' + (r.result.exceptionDetails.exception?.description ?? r.result.exceptionDetails.text);
  const v = r.result?.result?.value;
  return typeof v === 'string' ? v : JSON.stringify(v);
}

if (cmd === 'eval') console.log(await evaluate(arg));
else if (cmd === 'text') console.log(await evaluate('document.body.innerText'));
else if (cmd === 'shot') {
  const r = await send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(arg, Buffer.from(r.result.data, 'base64'));
  console.log('saved', arg);
}
ws.close();
