import {mkdir, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const [url, output, movie] = process.argv.slice(2);
const {chromium} = await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
await mkdir(output, {recursive: true});
const browser = await chromium.launch({executablePath: process.env.RETROM_CHROME_EXECUTABLE, headless: true});
const page = await browser.newPage();
const errors = [];
page.on('pageerror', e => errors.push(e.stack));
page.on('crash', () => errors.push('Browser renderer crashed'));
const bounded = async promise => {
  let timer;
  try { return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('Movie operation exceeded 120s')),120000);})]); }
  finally {clearTimeout(timer);}
};
const samples = [];
let maxWasmBytes = 0;
let maxVideoBytes = 0;
let maxQueuedVideoBytes = 0;
let maxQueuedVideoFrames = 0;
try {
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector('#start')?.disabled === false);
  await page.evaluate(() => { globalThis.movieProbe = createMovieProbe(); });
  const previewVideoBytes = await bounded(page.evaluate(movie => movieProbe.preview(movie), movie));
  if (!(previewVideoBytes > 0)) throw new Error('Preview is empty');
  await page.evaluate(() => movieProbe.stop());
  const fullAssetError = await page.evaluate(movie => movieProbe.full_asset_error(movie), movie);
  if (!fullAssetError.includes('full movie assets are unavailable on the Web')) throw new Error('Legacy full-frame API remains active');
  const poll = async (ms, loop = false) => {
    const value = await bounded(page.evaluate(({movie, ms, loop}) => ({
      ...movieProbe.poll(movie, ms, loop), wasmBytes: compatibility.wasmBytes,
    }), {movie, ms, loop}));
    maxWasmBytes = Math.max(maxWasmBytes, value.wasmBytes);
    maxVideoBytes = Math.max(maxVideoBytes, value.videoBytes);
    maxQueuedVideoBytes = Math.max(maxQueuedVideoBytes, value.queuedVideoBytes || 0);
    maxQueuedVideoFrames = Math.max(maxQueuedVideoFrames, value.queuedVideoFrames || 0);
    if (value.queuedVideoFrames > 8 || value.queuedVideoBytes > value.width * value.height * 3 * 8) throw new Error('YUV reordering queue exceeded its fixed bound');
    if (value.videoFrames > 2 || value.cachedAssets !== 0) {
      throw new Error('Unbounded movie asset path used: ' + JSON.stringify({frames: value.videoFrames, assets: value.cachedAssets}));
    }
    if (value.videoBytes > value.width * value.height * 4 * 2) throw new Error('Decoded frame bound exceeded');
    return value;
  };
  const first = await poll(0);
  let duration = first.durationMs;
  if (previewVideoBytes !== first.width * first.height * 4) throw new Error('Preview decoded more than one frame');
  if (first.frameIndex !== 0) throw new Error('Invalid start frame');
  const checksums = new Set([first.checksum]);
  samples.push({ms: 0, ...first});
  for (let ms = 100; ms <= (duration > 0 ? duration + 100 : 600000); ms += 100) {
    const value = await poll(ms);
    if (value.clampedMs >= 0) throw new Error('Decoder did not keep up with 100ms polling');
    if (!duration && value.durationMs > 0) duration = value.durationMs;
    checksums.add(value.checksum);
    if (ms % 5000 === 0 || ms >= duration) samples.push({ms, ...value});
    if (ms % 10000 === 0) console.log(JSON.stringify({progressMs: ms, maxWasmBytes, maxVideoBytes}));
  }
  if (!(duration > 0)) throw new Error('Movie duration was not discovered at EOF');
  if (checksums.size < 2) throw new Error('Video did not visually advance');
  const looped = await poll(duration, true);
  if (looped.frameIndex !== 0 || looped.checksum !== first.checksum) throw new Error('Loop did not return to the first frame');
  for (let ms = 100; ms <= 2000; ms += 100) await poll(duration + ms, true);
  const reopenWasmBytes = [];
  for (let cycle = 0; cycle < 3; cycle++) {
    const activeAfter120 = await page.evaluate(() => movieProbe.idle(120));
    const activeAfter121 = await page.evaluate(() => movieProbe.idle(1));
    if (activeAfter120 !== 1 || activeAfter121 !== 0) throw new Error('Idle stream lifetime did not release the decoder');
    const restarted = await poll(0);
    if (restarted.frameIndex !== 0 || restarted.checksum !== first.checksum) throw new Error('Evicted movie did not reopen at its first frame');
    reopenWasmBytes.push(restarted.wasmBytes);
  }
  const readBytes = await page.evaluate(() => Math.max(0,...compatibility.reads.map(r=>r.bytes)));
  if (reopenWasmBytes[2] > reopenWasmBytes[1] + Math.max(4*1024*1024,readBytes/4)) throw new Error('Repeated reopen memory did not settle');
  const stoppedBytes = await page.evaluate(() => movieProbe.stop());
  if (stoppedBytes !== 0 || errors.length) throw new Error('Resources not released or browser errors remain');
  const result = {status: 'PASS', movie, durationMs: duration, maxWasmBytes,
    maxVideoBytes, maxQueuedVideoBytes, maxQueuedVideoFrames, previewVideoBytes, visualChecksums: checksums.size, stoppedVideoBytes: stoppedBytes, idleEviction: true, reopenWasmBytes, samples};
  await writeFile(resolve(output, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify({...result, samples: samples.length}));
} catch (error) {
  await writeFile(resolve(output, 'result.json'), JSON.stringify({status: 'FAIL', movie,
    reason: String(error), maxWasmBytes, maxVideoBytes, errors, samples}, null, 2));
  console.log(JSON.stringify({status: 'FAIL', reason: String(error), maxWasmBytes}));
  process.exitCode = 1;
} finally { await browser.close(); }
