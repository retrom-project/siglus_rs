import {mkdir, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const [url, output, x, y] = process.argv.slice(2);
const {chromium} = await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
await mkdir(output, {recursive: true});
const browser = await chromium.launch({executablePath: process.env.RETROM_CHROME_EXECUTABLE,
  headless: true, args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const page = await browser.newPage({viewport: {width: 1280, height: 790}});
const messages = [];
page.on('console', message => messages.push({type: message.type(), text: message.text()}));
page.on('pageerror', error => messages.push({type: 'pageerror', text: String(error), stack: error.stack}));
const state = () => page.evaluate(() => ({...globalThis.compatibility, probe: globalThis.__siglusCompatProbe}));
const snapshot = async name => {
  await page.screenshot({path: resolve(output, name + '.png')});
  await writeFile(resolve(output, name + '-state.json'), JSON.stringify(await state(), null, 2));
  await writeFile(resolve(output, 'browser-log.json'), JSON.stringify(messages, null, 2));
};
try {
  await page.goto(url);
  await page.locator('#start').click({timeout: 30000});
  await page.waitForTimeout(1500);
  const openingDeadline = Date.now() + 600000;
  let lastProgress = Date.now();
  while (Date.now() < openingDeadline) {
    const current = await state();
    if (current.probe?.texts) break;
    if (messages.some(m => m.type === 'pageerror' || /panicked|step failed/.test(m.text))) {
      throw new Error('Core error before dialogue');
    }
    if (Date.now() - lastProgress > 15000) {
      console.log(JSON.stringify({opening: current.probe?.status, wasmBytes: current.wasmBytes,
        movieBytes: current.probe?.movieVideoBytes, movieFrames: current.probe?.movieVideoFrames}));
      await snapshot('opening-progress');
      lastProgress = Date.now();
    }
    // User input skips ordinary opening waits and selects the game's Start item.
    await page.mouse.click(Number(x), Number(y), {delay: 700});
    await page.waitForTimeout(600);
    if (!(await state()).probe?.texts) await page.keyboard.press('Enter', {delay: 700});
  }
  const first = await state();
  await snapshot('01-first-dialogue');
  if (!first.probe?.texts) throw new Error('No dialogue reached with real Start input');
  if (!first.reads.some(r => r.resolved === 'scene.chs')) throw new Error('Translated scenario was not loaded');
  const observed = new Map([[first.probe.status, first.probe.texts]]);
  for (let i = 0; i < 12 && observed.size < 3; i++) {
    await page.mouse.click(800, 500, {delay: 700});
    await page.waitForTimeout(1000);
    const next = await state();
    if (next.probe?.texts) observed.set(next.probe.status, next.probe.texts);
  }
  await snapshot('02-after-input');
  const failures = messages.filter(m => m.type === 'pageerror' || /panicked|step failed|WebGL: INVALID_/.test(m.text));
  const distinctDialogues = new Set(observed.values()).size;
  if (observed.size < 2 || distinctDialogues < 2) throw new Error('Input did not advance dialogue');
  if (failures.length) throw new Error('Browser/core errors remain: ' + failures[0].text);
  await writeFile(resolve(output, 'result.json'), JSON.stringify({status: 'LIMITED_DIALOGUE_PASS',
    scope: 'Standalone short dialogue probe; not whole-game or Retrom product acceptance',
    distinctDialogues, scriptLocations: observed.size, messages: messages.length,
    warnings: messages.filter(m => m.type === 'warning')}, null, 2));
  console.log(JSON.stringify({status: 'LIMITED_DIALOGUE_PASS', distinctDialogues, messages: messages.length}));
} catch (error) {
  await snapshot('failure');
  await writeFile(resolve(output, 'result.json'), JSON.stringify({status: 'FAIL', reason: String(error)}, null, 2));
  console.log(JSON.stringify({status: 'FAIL', reason: String(error)}));
  process.exitCode = 1;
} finally { await browser.close(); }
