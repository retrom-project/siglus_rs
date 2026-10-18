import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
const [url,output] = process.argv.slice(2);
const {chromium}=await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
await mkdir(output,{recursive:true});
const browser=await chromium.launch({executablePath:process.env.RETROM_CHROME_EXECUTABLE,headless:true});
const page=await browser.newPage();
const errors=[];page.on('pageerror',e=>errors.push(String(e)));
try {
  await page.goto(url);
  await page.waitForFunction(()=>!document.querySelector('#start').disabled);
  const checks=await page.evaluate(async()=>{
    const {SiglusSaveStore}=await import('/save-store.js');
    const base={exists:p=>p==='base.sav'||p==='savedata/original.sav',read:()=>new Uint8Array([9]),list:p=>p==='savedata'?['original.sav']:p?[]:['base.sav','savedata']};
    const failures=[];
    const store=await SiglusSaveStore.open('storage-regression',base,e=>failures.push(String(e)));
    const require=(yes,message)=>{if(!yes)throw new Error(message);};
    const bytes=new Uint8Array([0,255,128,42]);
    store.write('./SaveData/A.SAV',bytes); bytes.fill(0);
    require(store.read('savedata/a.sav')[1]===255,'write retained a mutable wasm view');
    store.rename('savedata/a.sav','savedata/b.sav');
    require(!store.exists('savedata/a.sav')&&store.exists('savedata/b.sav'),'rename did not update synchronous reads');
    require(store.list('savedata').includes('b.sav')&&!store.list('savedata').includes('a.sav'),'directory index stale after rename');
    store.remove('base.sav');
    require(!store.exists('base.sav'),'removed base file reappeared');
    store.write('savedata/config.sav',new Uint8Array([1]));
    store.write('savedata/config.sav',new Uint8Array([2]));
    const archive=await store.export(); // includes a flush barrier
    require(archive.files.some(f=>f.path==='savedata/original.sav'&&f.data==='CQ=='),'export omitted original read-only PC saves');
    await store.close();
    const reopened=await SiglusSaveStore.open('storage-regression',base);
    require(reopened.read('savedata/config.sav')[0]===2,'latest queued transaction was lost');
    require(reopened.read('savedata/b.sav')[1]===255&&!reopened.exists('base.sav'),'reload lost bytes/deletion');
    for(const invalid of [
      {...archive,scope:'another-game'},
      {...archive,files:[...archive.files,{path:'../escape.sav',data:'AA=='}]},
      {...archive,files:[...archive.files,archive.files[0]]},
    ]) {
      let rejected=false;try{await reopened.import(invalid);}catch{rejected=true;}
      require(rejected,'invalid archive was accepted');
    }
    require(JSON.stringify(await reopened.export())===JSON.stringify(archive),'failed import changed live saves');
    const isolated=await SiglusSaveStore.open('another-game',base);
    require(!isolated.exists('savedata/b.sav')&&isolated.exists('base.sav'),'games share their writable overlay');
    await isolated.close(); await reopened.close();
    const failing=await SiglusSaveStore.open('storage-regression',base,e=>failures.push(String(e)));
    failing.db.close(); // deterministic IndexedDB transaction creation failure
    failing.write('savedata/lost.sav',new Uint8Array([3]));
    let rejected=false;try{await failing.flush();}catch{rejected=true;}
    require(rejected&&failures.length===1,'persistence failure was swallowed');
    rejected=false;try{await failing.export();}catch{rejected=true;}
    require(rejected,'failed persistence reported a successful export');
    rejected=false;try{failing.write('savedata/later.sav',new Uint8Array([4]));}catch{rejected=true;}
    require(rejected,'new writes proceeded after a durable storage failure');
    const unchanged=await SiglusSaveStore.open('storage-regression',base);
    require(!unchanged.exists('savedata/lost.sav'),'failed transaction partially committed');
    await unchanged.close();
    return {copyFromWasm:true,includesOriginalSaves:true,renameAndListing:true,deletionPersists:true,writeOrdering:true,
      invalidImportAtomic:true,gameIsolation:true,failedFlushRejectsExport:true};
  });
  // This is the original Rust VFS seam, not a JavaScript-only surrogate.
  await page.evaluate(()=>runFilesystemProbe());
  await page.evaluate(()=>siglusSaves.flush());
  assert.equal(errors.length,0);
  await writeFile(resolve(output,'result.json'),JSON.stringify({status:'PASS',checks,rustFilesystemRoundtrip:true},null,2));
  console.log(JSON.stringify({status:'PASS',checks}));
}catch(error){
  await writeFile(resolve(output,'result.json'),JSON.stringify({status:'FAIL',reason:String(error),errors},null,2));
  console.log(JSON.stringify({status:'FAIL',reason:String(error)}));process.exitCode=1;
}finally{await browser.close();}
