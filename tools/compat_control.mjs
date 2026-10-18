import {mkdir, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createInterface} from 'node:readline';

const [url, output] = process.argv.slice(2);
const {chromium} = await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
await mkdir(output, {recursive: true});
const browser = await chromium.launch({
  executablePath: process.env.RETROM_CHROME_EXECUTABLE, headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const page = await browser.newPage({viewport: {width: 1280, height: 790}});
const messages = [];
page.on('console', message => messages.push({type: message.type(), text: message.text()}));
page.on('pageerror', error => messages.push({type: 'pageerror', text: String(error), stack: error.stack}));
await page.goto(url);
await page.locator('#start').click({timeout: 30000});
console.log(JSON.stringify({ready: true}));
const input = createInterface({input: process.stdin, terminal: false});
for await (const line of input) {
  try {
    const command = JSON.parse(line);
    if (command.action === 'wait') await page.waitForTimeout(command.ms);
    else if (command.action === 'key') await page.keyboard.press(command.key, {delay: command.delay ?? 100});
    else if (command.action === 'click') await page.mouse.click(command.x, command.y,
      {delay: command.delay ?? 100, button: command.button ?? 'left'});
    else if (command.action === 'untilDialogue') {
      const deadline = Date.now() + (command.timeoutMs ?? 600000);
      while (Date.now() < deadline) {
        if (await page.evaluate(() => Boolean(globalThis.__siglusCompatProbe?.texts))) break;
        if (messages.some(m => m.type === 'pageerror' || /panicked|step failed/.test(m.text))) throw new Error('Core error');
        await page.mouse.click(command.x, command.y, {delay: 700});
        await page.waitForTimeout(600);
        if (!(await page.evaluate(() => Boolean(globalThis.__siglusCompatProbe?.texts)))) {
          await page.keyboard.press('Enter', {delay: 700});
        }
      }
      if (!(await page.evaluate(() => Boolean(globalThis.__siglusCompatProbe?.texts)))) throw new Error('Dialogue timeout');
    }
    else if (command.action === 'move') await page.mouse.move(command.x, command.y);
    else if (command.action === 'evaluate') console.log(JSON.stringify(await page.evaluate(command.expression)));
    else if (command.action === 'snap') {
      await page.screenshot({path: resolve(output, command.name + '.png')});
      await writeFile(resolve(output, command.name + '-state.json'), JSON.stringify(await page.evaluate(() => ({...globalThis.compatibility, probe: globalThis.__siglusCompatProbe})), null, 2));
      await writeFile(resolve(output, command.name + '-log.json'), JSON.stringify(messages, null, 2));
    } else if (command.action === 'state') {
      console.log(JSON.stringify(await page.evaluate(() => ({...globalThis.compatibility,
        reads: globalThis.compatibility.reads.slice(-8), probe: globalThis.__siglusCompatProbe}))));
    } else if (command.action === 'log') console.log(JSON.stringify(messages.slice(-command.count || -12)));
    else if (command.action === 'close') break;
    else throw new Error('Unknown action');
    console.log(JSON.stringify({done: command.action, messageCount: messages.length}));
  } catch (error) { console.log(JSON.stringify({error: String(error)})); }
}
input.close();
process.stdin.destroy();
await browser.close();
