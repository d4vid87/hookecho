import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import {fixtureRadar} from './fixtures/radar.mjs';
const root=process.env.HOOKECHO_SOAK_OUTPUT??'outputs/soak';
await fs.mkdir(root,{recursive:true});
const baseUrl=process.env.HOOKECHO_BASE_URL??'http://127.0.0.1:8765/';
const duration=Number(process.env.HOOKECHO_SOAK_SECONDS??10800)*1000;
const browser=await puppeteer.launch({executablePath:process.env.CHROME_BIN??'/usr/bin/chromium',headless:true,args:['--no-sandbox','--enable-unsafe-swiftshader','--use-gl=angle','--use-angle=swiftshader']});
const report={started:new Date().toISOString(),durationMs:duration,samples:[],errors:[],requests:0,duplicateConcurrent:[],actions:[],fixtureDescription:'Repeated historical KDMX sweep bytes under six synthetic timestamp/station request names; no live radar or network latency claim.'};
try {
 const page=await browser.newPage(); await page.setViewport({width:1440,height:1000}); await page.setBypassServiceWorker(true);
 const state={offline:false}; report.fixture=await fixtureRadar(page,{loopFrames:true,networkState:state});
 report.navigations=[];page.on('framenavigated',frame=>{if(frame===page.mainFrame())report.navigations.push({time:Date.now(),url:frame.url()});});const active=new Map(); page.on('request',request=>{report.requests++;const url=request.url();if(active.has(url))report.duplicateConcurrent.push({url,time:Date.now()});active.set(url,request);});
 const done=request=>{if(active.get(request.url())===request)active.delete(request.url());};page.on('requestfinished',done);page.on('requestfailed',done);
 page.on('pageerror',error=>report.errors.push({time:Date.now(),error:error.message}));page.on('console',message=>{if(/panicked at|RuntimeError: unreachable/.test(message.text()))report.errors.push({time:Date.now(),error:message.text()});});
 const pane=i=>({site:'KDMX',moment:'Reflectivity',smooth:i%2===1,tilt:0,basemap:i%2?'light':'dark',lon:-93.72,lat:41.73,zoom:6,appearance:{precision:i%2===0},fields_on:[],globe:i===3});
 const workspace=n=>({name:`Soak ${n}`,panes:Array.from({length:n},(_,i)=>pane(i)),active:0,link_cameras:false,link_times:true,overlays_on:[],fields_on:[],adopt_site:false,chrome:{analyst_open:true,analyst_inspector_open:false}});
 await page.evaluateOnNewDocument(settings=>{Object.defineProperty(navigator,'gpu',{value:undefined});if(!localStorage.getItem('hookecho-settings'))localStorage.setItem('hookecho-settings',JSON.stringify(settings)); window.soakCadence=[];let previous;function tick(now){if(previous&&document.visibilityState==='visible'){window.soakCadence.push(now-previous);if(window.soakCadence.length>10000)window.soakCadence.shift();}previous=now;requestAnimationFrame(tick);}requestAnimationFrame(tick);document.addEventListener('visibilitychange',()=>{previous=undefined;});window.soakRecoveryEvents=[];window.addEventListener('hookecho-device-lost',()=>window.soakRecoveryEvents.push('device-lost'));document.addEventListener('webglcontextlost',()=>window.soakRecoveryEvents.push('webglcontextlost'),true);}, {setup_done:true,seeded_workspaces:true,default_site:'KDMX',radar_feed:'ArchiveOnly',workspaces:[workspace(2),workspace(4)]});
 const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms)); const ready=()=>page.waitForFunction(()=>!document.getElementById('boot'),{timeout:90000});
 await page.goto(baseUrl,{waitUntil:'domcontentloaded'});await ready();await wait(2000);
 const cdp=await page.createCDPSession(); await cdp.send('Performance.enable'); const system=await browser.target().createCDPSession();
 async function command(text){await page.keyboard.down('Control');await page.keyboard.press('K');await page.keyboard.up('Control');await wait(400);await page.keyboard.down('Control');await page.keyboard.press('A');await page.keyboard.up('Control');await page.keyboard.type(text,{delay:30});await wait(800);await page.keyboard.press('Enter');await wait(1500);}
 let count=2;await command('Soak 2');await page.keyboard.press('Escape');await page.keyboard.press('Space');await wait(2000);
 const started=Date.now();let nextSwitch=duration/4;let nextDisruption=duration/6;let round=0;
 while(Date.now()-started<duration){
  const elapsed=Date.now()-started;
  if(elapsed>=nextSwitch){count=count===2?4:2;await command(`Soak ${count}`);await page.keyboard.press('Escape');await page.keyboard.press('Space');report.actions.push({elapsed,kind:'layout',panes:count});nextSwitch+=duration/4;}
  if(elapsed>=nextDisruption){state.offline=true;report.actions.push({elapsed,kind:'network-interruption'});await command(round++%2?'KDMX':'KTLX');await wait(Math.min(10000,duration/10));state.offline=false;report.actions.push({elapsed:Date.now()-started,kind:'network-restored'});await command('Play');const other=await browser.newPage();await other.goto('about:blank');await other.bringToFront();report.actions.push({elapsed:Date.now()-started,kind:'background',visibility:await page.evaluate(()=>document.visibilityState)});await wait(Math.min(10000,duration/10));await page.bringToFront();await other.close();report.actions.push({elapsed:Date.now()-started,kind:'foreground'});nextDisruption+=duration/6;}
  let checkpoint=await page.evaluate(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1')));if(!checkpoint.playing.some(Boolean)){await page.keyboard.press('Escape');await wait(200);await page.keyboard.press('Escape');await wait(200);await page.mouse.click(500,600);await page.keyboard.press('Space');await page.waitForFunction(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.playing.some(Boolean),{timeout:15000});checkpoint=await page.evaluate(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1')));}assert(checkpoint.playing.some(Boolean),'playback remains active'); assert.equal(checkpoint.session.workspace.panes.length,count);
  const metrics=Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(metric=>[metric.name,metric.value]));
  const cadence=await page.evaluate(()=>{const values=window.soakCadence.slice().sort((a,b)=>a-b);window.soakCadence=[];return {samples:values.length,p95:values[Math.floor(values.length*.95)]??null,over50ms:values.filter(value=>value>50).length,visibility:document.visibilityState,recoveryEvents:window.soakRecoveryEvents};});
  let rss=0;for(const process of (await system.send('SystemInfo.getProcessInfo')).processInfo){try{const text=await fs.readFile(`/proc/${process.id}/status`,'utf8');rss+=Number(text.match(/VmRSS:\s+(\d+)/)?.[1]??0)*1024;}catch{}}
  report.samples.push({elapsed:Date.now()-started,panes:count,playing:checkpoint.playing,times:checkpoint.session.panes.map(pane=>pane.time),heapUsed:metrics.JSHeapUsedSize,taskDuration:metrics.TaskDuration,aggregateProcessRss:rss,cadence,url:page.url(),requests:report.requests});
  await fs.writeFile(root+'/hookecho-next-soak-progress.json',JSON.stringify(report,null,2));
  if(report.errors.length)throw new Error(JSON.stringify(report.errors));
  await wait(Math.min(30000,duration/12));
 }
 report.finished=new Date().toISOString();report.elapsedMs=Date.now()-started;
 await page.screenshot({path:root+'/hookecho-next-soak.png'});
 report.bundle=await page.evaluate(()=>performance.getEntriesByType('resource').filter(entry=>entry.name.includes('hookecho_bg-')).map(entry=>entry.name));
 await fs.writeFile(root+'/hookecho-next-soak.json',JSON.stringify(report,null,2));console.log(JSON.stringify({passed:true,elapsedMs:report.elapsedMs,samples:report.samples.length,errors:report.errors.length}));
} finally {await browser.close();}
