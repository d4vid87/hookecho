import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import {fixtureRadar} from './fixtures/radar.mjs';
const output=process.env.HOOKECHO_RELIABILITY_OUTPUT??'outputs/reliability';
await fs.mkdir(output,{recursive:true});
const browser=await puppeteer.launch({executablePath:process.env.CHROME_BIN??puppeteer.executablePath(),headless:true,args:['--no-sandbox','--enable-unsafe-swiftshader','--use-gl=angle','--use-angle=swiftshader']});
const report={started:new Date().toISOString(),fixture:'Historical KDMX sweep bytes under synthetic station/time names. No live-feed or real-device latency claim.',errors:[],checks:{}};
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
let page;
try {
 page=await browser.newPage();await page.setViewport({width:1440,height:1000});await page.setBypassServiceWorker(true);
 const network={offline:false,failFrame:'170300',delayMs:400};report.requests=await fixtureRadar(page,{loopFrames:true,networkState:network});
 page.on('pageerror',error=>report.errors.push(error.message));
 page.on('console',message=>{if(/panicked at|RuntimeError: unreachable/.test(message.text()))report.errors.push(message.text());});
 const pane=(i,site)=>({site,moment:'Reflectivity',smooth:i%2===1,tilt:0,basemap:i%2?'light':'dark',lon:-93.72,lat:41.73,zoom:6,appearance:{precision:i%2===0},fields_on:[],globe:false});
 const workspace=(n,site)=>({name:`Reliability ${n} ${site}`,panes:Array.from({length:n},(_,i)=>pane(i,site)),active:0,link_cameras:false,link_times:false,overlays_on:[],fields_on:[],adopt_site:false,chrome:{analyst_open:true,analyst_inspector_open:n===2&&site==='KTLX'}});
 await page.evaluateOnNewDocument(settings=>{
  Object.defineProperty(navigator,'gpu',{value:undefined});
  localStorage.setItem('hookecho-settings',JSON.stringify(settings));
  window.reportBlobs=[];const create=URL.createObjectURL.bind(URL);
  URL.createObjectURL=blob=>{blob.text().then(text=>{try{const object=JSON.parse(text);if(object.schema==='hookecho.diagnostics/v2')window.reportBlobs.push(object);}catch{}});return create(blob);};
 },{setup_done:true,seeded_workspaces:true,default_site:'KDMX',radar_feed:'ArchiveOnly',workspaces:[workspace(2,'KDMX'),workspace(4,'KDMX'),workspace(2,'KTLX')],mapbox_key:'private-provider-secret',markers:[{id:'private-marker-id',name:'private-marker-name',lat:41.712345,lon:-93.654321,home:false}]});
 await page.goto(process.env.HOOKECHO_BASE_URL??'http://127.0.0.1:8765/',{waitUntil:'domcontentloaded'});
 await page.waitForFunction(()=>!document.getElementById('boot'),{timeout:90000});await page.waitForFunction(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.panes[0]?.time,{timeout:90000});await wait(3000);
 // Focus the canvas, then use the search shortcut; toolbar coordinates change with layout.
 // CommandSearch requests text-field focus, so no fixed drawer coordinates are needed.
 async function command(text){
  await page.mouse.click(702,202);await wait(200);
  await page.keyboard.down('Control');await page.keyboard.press('K',{delay:150});await page.keyboard.up('Control');await wait(700);
  await page.keyboard.down('Control');await page.keyboard.press('A',{delay:150});await page.keyboard.up('Control');
  await page.keyboard.type(text,{delay:30});await wait(600);await page.keyboard.press('Enter',{delay:150});await wait(700);
  await page.mouse.click(702,202);await wait(250);
 }

 async function checkpoint(){return page.evaluate(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1')));}
 async function diagnostics(){const count=await page.evaluate(()=>window.reportBlobs.length);await command('Save diagnostics');await page.waitForFunction(count=>window.reportBlobs.length>count,{timeout:10000},count);return page.evaluate(()=>window.reportBlobs.at(-1));}
 async function select(n,site){await command(`Reliability ${n} ${site}`);await page.waitForFunction((n,site)=>{const cp=JSON.parse(localStorage.getItem('hookecho.recovery.v1'));return cp?.session.workspace.panes.length===n&&cp.session.workspace.panes.every(p=>p.site===site);},{timeout:15000},n,site);}
 async function ready(n,site){await page.waitForFunction(()=>window.reportBlobs.length>=0);for(let attempt=0;attempt<30;attempt++){const diag=await diagnostics();if(diag.reproduction.panes.length===n&&diag.reproduction.panes.every(p=>p.selected_station===site&&!p.station_transition&&p.volume_time)&&diag.render_metrics.panes.every(p=>p.radar_upload_requested>0&&p.radar_upload_requested===p.radar_upload_prepared))return diag;await wait(1000);}assert.fail('radar panes did not become ready');}
 await select(4,'KDMX');const four=await ready(4,'KDMX');report.four=four;assert(four.shared_downloads.joined>0,'matching pane requests must join a shared raw download');report.checks.sharedDownloads=true;
 assert(four.timeline_availability.includes('Ready'));report.checks.readyTimeline=true;
 await command('Jump to live');
 let diag=await diagnostics();if(diag.reproduction.panes[0].playing){await page.keyboard.press('Space');await wait(500);}
 // Listings may contain duplicate timestamps; choose the failing time rather than assuming
 // two backward steps always reach it. Keep the unavailable-frame assertion below unchanged.
 for(let step=0;step<24;step++){
  const cp=await checkpoint();if(cp.session.panes[0].time?.includes('T17:03:00'))break;
  await page.keyboard.press('ArrowLeft');await wait(1200);
 }
 assert((await checkpoint()).session.panes[0].time?.includes('T17:03:00'),'must select the deliberately failing radar frame');
 await wait(1500);
 const unavailable=await diagnostics();assert(unavailable.timeline_availability.includes('Unavailable'),'failed selected frame must be marked unavailable');report.unavailable=unavailable;await page.screenshot({path:output+'/timeline-unavailable.png'});
 await page.keyboard.press('Space');await wait(2000);const skipped=await diagnostics();assert(skipped.timeline_availability.includes('Skipped'),'playback must identify skipped unavailable frame');report.skipped=skipped;report.checks.unavailableAndSkippedTimeline=true;
 await command('Jump to live');await wait(500);
 const before=await checkpoint();await command('2 panes');await page.waitForFunction(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.workspace.panes.length===2);
 const two=await ready(2,'KDMX');report.two=two;assert(two.render_metrics.nominal_gpu_bytes<four.render_metrics.nominal_gpu_bytes,'closing panes must reduce nominal GPU allocation');report.checks.closedPaneMemory=true;
 await command('Undo workspace action');await page.waitForFunction(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.workspace.panes.length===4);const restored=await checkpoint();assert.deepEqual(restored.session.workspace.panes,before.session.workspace.panes);report.checks.closedPaneUndo=true;
 network.offline=true;await select(2,'KTLX');const retained=await diagnostics();report.retained=retained;assert(retained.reproduction.panes.every(p=>p.selected_station==='KTLX'&&p.displayed_station==='KDMX'&&p.station_transition&&p.volume_time),'old scan must remain explicitly identified while replacement is unavailable');report.retained=retained;await page.screenshot({path:output+'/station-loading.png'});report.checks.stationContinuity=true;
 await command('Undo workspace action');await page.waitForFunction(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.workspace.panes.length===4);const restoredWorkspace=await checkpoint();assert.deepEqual(restoredWorkspace.session.workspace.panes,before.session.workspace.panes);report.checks.workspaceUndo=true;
 network.offline=false;await ready(4,'KDMX');
 const final=await diagnostics();report.final=final;const text=JSON.stringify(final);for(const privateValue of ['private-provider-secret','private-marker-id','private-marker-name','41.712345','93.654321'])assert(!text.includes(privateValue),`diagnostics exposed ${privateValue}`);
 assert(final.recent_interactions.some(event=>event.action==='undo_workspace_action'));assert(final.recent_interactions.length<=64);assert(final.undo_history.entries<=16);assert(final.undo_history.serialized_bytes<=2*1024*1024);report.checks.diagnosticsPrivacyAndHistory=true;
 assert.equal(report.errors.length,0);await page.screenshot({path:output+'/final.png'});report.finished=new Date().toISOString();
} catch(error) {report.failure=error.message;if(page){report.checkpoint=await page.evaluate(()=>JSON.parse(localStorage.getItem('hookecho.recovery.v1')));await page.screenshot({path:output+'/failure.png'});}throw error;} finally {await fs.writeFile(output+'/report.json',JSON.stringify(report,null,2));await browser.close();}
console.log(JSON.stringify(report.checks));
