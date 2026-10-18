// Drive original game UI. Coordinates are test profiles, never engine patches.
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {resolve,dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';

const [url, output, profile, resumeArgument, seedArchiveFile] = process.argv.slice(2);
const resumeDirectory = resumeArgument && resumeArgument !== "-" ? resumeArgument : null;
const layouts = {
  hatsukoi: {start: [360,704], load: [500,704], saveSlot: [310,280], loadSlot: [310,280]},
  mashiro: {start: [1010,233], load: [1010,310], saveSlot: [250,205], loadSlot: [250,205]},
};
const layout = layouts[profile];
if (!layout) throw new Error('Unknown UI test profile');
const {chromium} = await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
await mkdir(output, {recursive: true});
const browser = await chromium.launch({executablePath: process.env.RETROM_CHROME_EXECUTABLE, headless: true,
  args: ['--no-sandbox','--enable-unsafe-swiftshader','--use-gl=angle','--use-angle=swiftshader',
    ...(process.env.RETROM_COMPAT_CDP_PORT ? ['--remote-debugging-port='+Number(process.env.RETROM_COMPAT_CDP_PORT)] : [])]});
const messages = [];
let page, context, phase = 'start', lastState;
const state = async () => lastState = await page.evaluate(() => ({...compatibility, reads: compatibility.reads.slice(-12), probe: globalThis.__siglusCompatProbe, saves: siglusSaves.list()}));
const click = async (x,y,button='left') => {
  const frames = () => page.evaluate(()=>compatibility.probeFrames||0);
  const waitFrames = async () => { const before=await frames(); await page.waitForFunction(n=>(compatibility.probeFrames||0)>n+1,before); };
  // Canvas buttons compute hover during engine frames. Move first, then hold
  // through two actual frames so software GPU stalls cannot swallow the click.
  await page.mouse.move(x,y); await waitFrames();
  await page.mouse.down({button}); await waitFrames();
  await page.mouse.up({button}); await waitFrames();
  await page.waitForTimeout(500);
};
const fatal = () => messages.find(m => m.type === 'pageerror' || /panicked|step failed|WebGL: INVALID_/.test(m.text));
const snapshot = async name => {
  await page.screenshot({path:resolve(output,name+'.png')});
  await writeFile(resolve(output,name+'.json'),JSON.stringify(await state(),null,2));
  await writeFile(resolve(output,'browser-log.json'),JSON.stringify(messages,null,2));
};
async function fresh(archive) {
  context = await browser.newContext({viewport:{width:1280,height:790}});
  page = await context.newPage();
  page.on('console',m=>messages.push({phase,type:m.type(),text:m.text()}));
  page.on('pageerror',e=>messages.push({phase,type:'pageerror',text:String(e),stack:e.stack}));
  page.on('crash',()=>messages.push({phase,type:'pageerror',text:'Browser renderer crashed'}));
  page.on('close',()=>messages.push({phase,type:'close',text:'Page closed'}));
  await page.goto(url);
  await page.waitForFunction(()=>!document.querySelector('#start').disabled);
  if ((await state()).saves.length) throw new Error('Fresh browser context unexpectedly already has saves');
  if (archive) await page.evaluate(a=>siglusSaves.import(a),archive);
  await page.locator('#start').click();
}
async function until(condition, action, timeout=600000) {
  const end=Date.now()+timeout; let report=Date.now();
  while(Date.now()<end) {
    const current=await state();
    if (condition(current)) return current;
    if(fatal()) throw new Error(fatal().text);
    if(Date.now()-report>20000) { console.log(JSON.stringify({phase,status:current.probe?.status,wasmBytes:current.wasmBytes})); await snapshot('progress'); report=Date.now(); }
    if(action) await action(); else await page.waitForTimeout(500);
  }
  throw new Error('Timeout in '+phase);
}
const digests = archive => archive.files.filter(f=>f.data!==null).map(f=>({path:f.path,
  bytes:Buffer.from(f.data,'base64').length, sha256:createHash('sha256').update(Buffer.from(f.data,'base64')).digest('hex')})).sort((a,b)=>a.path.localeCompare(b.path));
try {
  let checkpoint, archive, files;
  if (resumeDirectory) {
    checkpoint=JSON.parse(await readFile(resolve(resumeDirectory,'01-checkpoint.json'),'utf8'));
    archive=JSON.parse(await readFile(resolve(resumeDirectory,'save-archive.json'),'utf8'));
    files=digests(archive);
  } else {
  const seed = seedArchiveFile ? JSON.parse(await readFile(seedArchiveFile,'utf8')) : null;
  await fresh(seed);
  phase=seed?'load-existing-native-save':'opening';
  if(seed) {
    await until(s=>/(_01load|_sys_menu_(sl|lo)0[01])/.test(s.probe?.status||''),()=>click(...layout.load),90000);
    const seedCheckpoint=JSON.parse(await readFile(resolve(dirname(seedArchiveFile),'01-checkpoint.json'),'utf8'));
    const seedScene=seedCheckpoint.probe.status.match(/scene=(.*?) line=/)[1];
    await until(s=>Boolean(s.probe?.texts)&&s.probe.status.includes('scene='+seedScene+' '),async()=>{
      await click(...layout.loadSlot);
      await click(...(profile==='hatsukoi'?[540,682]:[570,430]));
    },90000);
  }
  phase='opening';
  await until(s=>Boolean(s.probe?.texts),async()=>{
    await click(...layout.start);
    if (!(await state()).probe?.texts) await page.keyboard.press('Enter',{delay:500});
  });
  const first=await state();
  phase='advance-before-save';
  const advanced=await until(s=>Boolean(s.probe?.texts)&&s.probe.status!==first.probe.status,()=>click(800,500),60000);
  await page.waitForTimeout(2500);
  checkpoint=await state();
  await snapshot('01-checkpoint');
  const writesBeforeSave = checkpoint.writes.length;
  phase='save';
  if(profile==='hatsukoi') { await click(800,500,'right'); await click(775,714); }
  else { await click(740,769); await page.waitForTimeout(2000); }
  await snapshot('02-save-menu');
  if(profile==='mashiro') await click(800,500); // first-use menu explanation
  await click(...layout.saveSlot);
  if(profile==='mashiro') await click(570,430); // original YES confirmation
  await until(s=>s.writes.slice(writesBeforeSave).some(f=>/savedata\/\d{4}\.sav$/.test(f.path)),profile==='mashiro'?async()=>{
    // Original first-use help consumes clicks before the slot becomes active.
    // Repeat only while no save exists; all actions remain inside its SAVE UI.
    await click(...layout.saveSlot); await click(570,430);
  }:()=>click(540,682),90000);
  await page.evaluate(()=>siglusSaves.flush());
  await page.waitForTimeout(1500);
  await snapshot('03-saved');
  if(fatal()) throw new Error(fatal().text);
  archive=await page.evaluate(()=>siglusSaves.export());
  files=digests(archive);
  for(const name of ['global.sav','read.sav','config.sav']) if(!files.some(f=>f.path.endsWith('/'+name))) throw new Error('Missing '+name);
  if(!files.some(f=>/\/\d{4}\.sav$/.test(f.path)&&f.bytes>10000)) throw new Error('No full local save payload');
  if(!files.some(f=>/\/\d{4}\.(bmp|png)$/.test(f.path)&&f.bytes>100)) throw new Error('Configured save thumbnail was not written');
  await writeFile(resolve(output,'save-archive.json'),JSON.stringify(archive));
  phase='durable-reload';
  await page.reload();
  await page.waitForFunction(()=>!document.querySelector('#start').disabled);
  const persisted=await page.evaluate(()=>siglusSaves.export());
  if(JSON.stringify(digests(persisted))!==JSON.stringify(files)) throw new Error('IndexedDB reload changed saved bytes');
  await context.close();
  }
  phase='new-instance-import';
  // A completely new browser context has neither a VM nor the first context's
  // IndexedDB. Only the exported save-file archive crosses this boundary.
  await fresh(archive);
  phase='load-menu';
  await until(s=>/(_01load|_sys_menu_(sl|lo)0[01])/.test(s.probe?.status||''),()=>click(...layout.load),90000);
  await snapshot('04-load-menu');
  phase='restore';
  await click(...layout.loadSlot);
  // Confirm using the game's actual dialog button. Enter is not mapped by
  // every original menu; blindly pressing it can also advance restored text.
  if(profile==='hatsukoi') await click(540,682);
  else await click(570,430);
  const scene=checkpoint.probe.status.match(/scene=(.*?) line=/)?.[1];
  const restored=await until(s=>s.probe?.status.includes('scene='+scene+' ')&&Boolean(s.probe?.texts),profile==='mashiro'?async()=>{
    const current=await state();
    if(/_sys_menu_(sl|lo)0/.test(current.probe?.status||'')) { await click(...layout.loadSlot); await click(570,430); }
    else await page.waitForTimeout(500);
  }:null,60000);
  await page.waitForTimeout(2000);
  await snapshot('05-restored');
  const loadedSavePoint=await state();
  // Siglus local saves resume the last SAVEPOINT (the beginning of a text
  // block), not the exact menu-opening instruction. Verify that original
  // continuation reaches the saved visible text, then continues past it.
  if(checkpoint.probe.savePoint && loadedSavePoint.probe.status!==checkpoint.probe.savePoint) {
    throw new Error('Did not restore the serialized SAVEPOINT: '+JSON.stringify({expected:checkpoint.probe.savePoint,actual:loadedSavePoint.probe.status}));
  }
  let replayInputs=0;
  await until(s=>s.probe?.status===checkpoint.probe.status&&s.probe?.texts===checkpoint.probe.texts,async()=>{
    if(++replayInputs>4) throw new Error('Restored SAVEPOINT did not reconnect to the saved text block');
    await click(800,500); await page.waitForTimeout(1000);
  },30000);
  const restoredFull=await state();
  await snapshot('05b-same-text-block');
  phase='advance-after-restore';
  const after=await until(s=>Boolean(s.probe?.texts)&&s.probe.status!==restoredFull.probe.status,()=>click(800,500),60000);
  await snapshot('06-after-input');
  if(fatal()) throw new Error(fatal().text);
  const result={status:'PASS',profile,checkpoint:checkpoint.probe,restored:restoredFull.probe,loadedSavePoint:loadedSavePoint.probe,replayInputs,
    after:after.probe,files,durableReload:!resumeDirectory,newContextImport:true,seededFromNativeSave:Boolean(seedArchiveFile),sourceGameFilesWritten:false};
  await writeFile(resolve(output,'result.json'),JSON.stringify(result,null,2));
  console.log(JSON.stringify({status:'PASS',profile,files:files.length,checkpoint:checkpoint.probe.status,restored:restoredFull.probe.status}));
} catch(error) {
  if(page&&!page.isClosed()) await snapshot('failure').catch(()=>{});
  await writeFile(resolve(output,'browser-log.json'),JSON.stringify(messages,null,2));
  await writeFile(resolve(output,'result.json'),JSON.stringify({status:'FAIL',profile,phase,reason:String(error),fatal:fatal(),lastState},null,2));
  console.log(JSON.stringify({status:'FAIL',profile,phase,reason:String(error)}));process.exitCode=1;
} finally {
  if(process.exitCode && process.env.RETROM_COMPAT_HOLD_ON_FAILURE) {
    console.log(JSON.stringify({heldForDiagnostics:true,cdpPort:process.env.RETROM_COMPAT_CDP_PORT}));
    await new Promise(()=>{});
  }
  await browser.close();
}
