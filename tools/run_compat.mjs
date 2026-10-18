import {mkdir, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const [url, output] = process.argv.slice(2);
if (!url || !output || !process.env.RETROM_PLAYWRIGHT_MODULE || !process.env.RETROM_CHROME_EXECUTABLE) {
  throw new Error('Set RETROM_PLAYWRIGHT_MODULE and RETROM_CHROME_EXECUTABLE; pass URL OUTPUT_DIRECTORY');
}
const {chromium} = await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
await mkdir(output, {recursive: true});
const browser = await chromium.launch({
  executablePath: process.env.RETROM_CHROME_EXECUTABLE,
  headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const page = await browser.newPage({viewport: {width: 1280, height: 790}});
const messages = [];
page.on('console', message => messages.push({type: message.type(), text: message.text()}));
page.on('pageerror', error => messages.push({type: 'pageerror', text: String(error)}));
page.on('requestfailed', request => messages.push({type: 'requestfailed', text: request.url(), failure: request.failure()}));
try {
  await page.goto(url);
  await page.locator('#start').click({timeout: 30000});
  await page.waitForTimeout(6000);
  await page.screenshot({path: resolve(output, '01-start.png')});
  console.log(JSON.stringify({phase: 'first-frame', messages: messages.slice(-8)}));
  await page.keyboard.press('Enter');
  await page.waitForTimeout(3500);
  await page.screenshot({path: resolve(output, '02-enter.png')});
  await writeFile(resolve(output, 'state.json'), JSON.stringify(await page.evaluate(() => globalThis.compatibility), null, 2));
  await writeFile(resolve(output, 'browser-log.json'), JSON.stringify(messages, null, 2));
  console.log(JSON.stringify({phase: 'after-enter', messages: messages.slice(-8)}));
} finally {
  await browser.close();
}
