// Reproducible camera/input trace. Vector payloads are captured once, then replayed unchanged.
// Requires the standard local web server; fixture interception provides radar and vector data.
import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import {fixtureRadar} from './fixtures/radar.mjs';
const out=process.env.HOOKECHO_MOTION_OUTPUT??'outputs/map-motion';
const cache=process.env.HOOKECHO_MOTION_TILES??'outputs/map-motion-tiles';
await fs.mkdir(out,{recursive:true});await fs.mkdir(cache,{recursive:true});
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const report={version:1,started:new Date().toISOString(),trials:[],errors:[],limits:['Browser scheduling and UI preparation, not GPU completion or presentation','Historical single-sweep radar fixture; recorded OpenFreeMap vector bytes','No reference app speed ranking; before/after use the same HookEcho scenes']};
const browser=await puppeteer.launch({executablePath:process.env.CHROME_BIN??puppeteer.executablePath(),headless:process.env.HEADFUL!=='1',args:['--no-sandbox',...(process.env.HEADFUL==='1'?['--use-gl=angle','--use-angle=gl']:['--enable-unsafe-swiftshader','--use-gl=angle','--use-angle=swiftshader'])]});
try {
 for(const [name,n,delay,linked,reduced] of [['warm',1,0],['cold',1,250],['quad',4,0],['quad-linked',4,0,true],['reduced',1,0,false,true]]) {
  const page=await browser.newPage();
  const inputSession=await page.createCDPSession();
  const wheelSequence=async()=>{const jobs=[];for(let i=0;i<40;i++){jobs.push(inputSession.send('Input.dispatchMouseEvent',{type:'mouseWheel',x:450,y:350,deltaX:0,deltaY:i<20?-24:24}));await wait(50)}await Promise.all(jobs);};
  await page.setViewport({width:1440,height:1000,deviceScaleFactor:1});await page.setBypassServiceWorker(true);
  page.on('pageerror',e=>report.errors.push(e.message));
  const pending=new Map();
  const vectorResponse=async url=>{
   const original=url.origin.includes('127.0.0.1')?'https://'+url.pathname.slice('/proxy/'.length)+url.search:url.href;
   const key=crypto.createHash('sha256').update(original).digest('hex');const file=cache+'/'+key;
   const read=async()=>{try{return await fs.readFile(file)}catch{assert(process.env.REPLAY_ONLY!=='1','missing recorded vector payload '+original);const r=await fetch(original);assert.equal(r.status,200);const bytes=Buffer.from(await r.arrayBuffer());await fs.writeFile(file,bytes);return bytes;}};
   if(!pending.has(key))pending.set(key,read());const body=await pending.get(key);if(delay)await wait(delay);
   return {status:200,contentType:url.pathname.endsWith('.pbf')?'application/x-protobuf':'application/json',body};
  };
  const requests=await fixtureRadar(page,{vectorResponse});
  const pane={site:'KDMX',moment:'Reflectivity',smooth:false,tilt:0,basemap:'dark',lon:-93.72,lat:41.73,zoom:6.45,appearance:{},fields_on:[],globe:false};
  const workspace={name:'Motion fixture',panes:Array.from({length:n},()=>pane),active:0,link_cameras:!!linked,link_times:false,overlays_on:[],fields_on:[],adopt_site:false,chrome:{analyst_open:true}};
  await page.evaluateOnNewDocument(s=>{
   globalThis.__motionGpuStats={draws:0,bufferWrites:0,textureWrites:0};
   for(const prototype of [WebGLRenderingContext.prototype,WebGL2RenderingContext.prototype]) {
    for(const [name,counter] of [['drawArrays','draws'],['drawElements','draws'],['drawArraysInstanced','draws'],['drawElementsInstanced','draws'],['bufferData','bufferWrites'],['bufferSubData','bufferWrites'],['texImage2D','textureWrites'],['texSubImage2D','textureWrites']]) {
     const original=prototype[name];if(!original)continue;prototype[name]=function(...args){__motionGpuStats[counter]++;return original.apply(this,args);};
    }
   }
   globalThis.__hookechoMapTraceEnabled=true;localStorage.clear();Object.defineProperty(navigator,'gpu',{value:undefined});localStorage.setItem('hookecho-settings',JSON.stringify(s));},{setup_done:true,reduce_motion:!!reduced,seeded_workspaces:true,default_site:'KDMX',radar_feed:'ArchiveOnly',workspaces:[workspace]});
  await page.goto(process.env.HOOKECHO_URL??'http://127.0.0.1:8765/',{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>!document.getElementById('boot'),{timeout:60000});await wait(2000);
  await page.mouse.click(248,38);await wait(400);await page.mouse.click(200,269);await page.keyboard.type('Motion fixture',{delay:30});await wait(500);await page.keyboard.press('Enter');await wait(700);await page.mouse.click(702,202);
  await page.waitForFunction(n=>new Set(globalThis.__hookechoMapTrace?.frames.map(f=>f[1])).size===n,{timeout:20000},n);
  if(!delay) {
   await wait(8000);await page.mouse.move(450,350);
   await wheelSequence();
   await wait(6000);
  } else await wait(100);
  await page.evaluate(()=>{globalThis.__hookechoMapTrace={version:1,frames:[],dropped:0};globalThis.__motionInput=[];globalThis.__motionGpuStats={draws:0,bufferWrites:0,textureWrites:0};addEventListener('wheel',e=>__motionInput.push([performance.now(),e.deltaY]),{passive:true,capture:true});});
  await page.mouse.move(450,350);
  await wheelSequence();await wait(1200);
  const trace=await page.evaluate(()=>{
   const gl=document.querySelector('canvas')?.getContext('webgl2');const ext=gl?.getExtension('WEBGL_debug_renderer_info');
   return {trace:__hookechoMapTrace,inputs:__motionInput,gpuCalls:__motionGpuStats,userAgent:navigator.userAgent,gpu:ext?gl.getParameter(ext.UNMASKED_RENDERER_WEBGL):null};
  });
  await fs.writeFile(out+'/'+name+'-trace.json',JSON.stringify(trace));
  assert(trace.gpuCalls.draws>0,'actual WebGL draw submissions required');
  assert.equal(trace.trace.dropped,0);assert.equal(trace.inputs.length,40);assert(trace.trace.frames.length>20);
  if(process.env.EXPECT_MOTION==='1') {
   const frames=new Map();for(const f of trace.trace.frames){const v=frames.get(f[2])??{uploads:0,moving:false};v.uploads+=f[12]+f[13];v.moving||=!!f[10];frames.set(f[2],v);}
   for(const f of frames.values())assert(f.uploads<=(f.moving?2:8),'upload budget multiplied across panes');
   for(const c of trace.trace.costs??[])assert(c[3]<=192,'optional label layout budget exceeded');
  }
  const main=trace.trace.frames.filter(f=>f[1]===0);assert(main.some(f=>f[3]!==main[0][3]),'wheel must change camera zoom');
  await page.screenshot({path:out+'/'+name+'.png'});
  const first=main[0];const anchor=[442,342];
  const world=[first[4]+(anchor[0]-first[6]/2)/(256*2**first[3]),first[5]+(anchor[1]-first[7]/2)/(256*2**first[3])];
  const drift=Math.max(...main.map(f=>Math.hypot((world[0]-f[4])*256*2**f[3]+f[6]/2-anchor[0],(world[1]-f[5])*256*2**f[3]+f[7]/2-anchor[1])));
  assert(drift<0.5,'cursor anchor drift '+drift);
  if(reduced && process.env.EXPECT_MOTION==='1') {
   const lastInput=trace.inputs.at(-1)[0];const tail=main.filter(f=>f[0]>lastInput+100);
   assert(tail.length>0);assert(tail.every(f=>Math.abs(f[3]-tail[0][3])<1e-5),'reduced motion must not retain a zoom tail');
  }
  const deltas=main.slice(1).map((f,i)=>f[0]-main[i][0]);const sorted=[...deltas].sort((a,b)=>a-b);const moving=main.slice(1).map((f,i)=>Math.abs(f[3]-main[i][3])>1e-6?f[0]-main[i][0]:null).filter(x=>x!==null).sort((a,b)=>a-b);const cpu=main.map(f=>f[15]).sort((a,b)=>a-b);const paneCpu=(trace.trace.costs??[]).filter(c=>c[1]===0).map(c=>c[4]).sort((a,b)=>a-b);
  report.trials.push({bundle:await page.evaluate(()=>performance.getEntriesByType('resource').filter(e=>/hookecho.*\.(js|wasm)/.test(e.name)).map(e=>e.name.split('?')[0])),name,panes:n,anchorDriftPx:drift,movingP50:moving[Math.floor(moving.length*.5)],movingP95:moving[Math.floor(moving.length*.95)],movingMax:moving.at(-1),gpuCalls:trace.gpuCalls,frames:main.length,p50:sorted[Math.floor(sorted.length*.5)],p95:sorted[Math.floor(sorted.length*.95)],max:sorted.at(-1),prepareP95:cpu[Math.floor(cpu.length*.95)],paneUiP95:paneCpu[Math.floor(paneCpu.length*.95)],movingFrames:main.slice(1).filter((f,i)=>f[3]!==main[i][3]).length,uploads:main.reduce((s,f)=>s+f[12]+f[13],0),vectorRequests:requests.vectorRequests.length,trace});
  console.log(JSON.stringify({...report.trials.at(-1),trace:undefined}));
  // Visual checks run after the timed window to avoid screenshot stalls contaminating timings.
  await page.mouse.wheel({deltaY:-160});await wait(100);await page.screenshot({path:out+'/'+name+'-transition.png'});
  await wait(1000);
  if(name==='warm') {
   await page.evaluate(()=>{globalThis.__hookechoMapTrace={version:1,frames:[],dropped:0};});
   await page.mouse.move(800,650);await page.mouse.down();
   for(let i=0;i<8;i++){await page.mouse.move(800+i*12,650);await wait(20)}
   const release=await page.evaluate(()=>performance.now());await page.mouse.up();await wait(900);
   const drag=await page.evaluate(()=>__hookechoMapTrace.frames);
   const coast=drag.filter(f=>f[0]>release+50&&f[0]<release+500);
   report.drag={coastFrames:coast.length,movedAfterRelease:coast.some(f=>Math.abs(f[4]-coast[0][4])>1e-8)};
   await fs.writeFile(out+'/drag-trace.json',JSON.stringify(drag));
  }
  if(name==='warm') {
   await page.evaluate(()=>{globalThis.__hookechoMapTrace={version:1,frames:[],dropped:0};});
   await inputSession.send('Emulation.setTouchEmulationEnabled',{enabled:true,maxTouchPoints:2});
   await inputSession.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:550,y:400,id:1},{x:650,y:400,id:2}]});await wait(80);
   await inputSession.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:530,y:400,id:1},{x:670,y:400,id:2}]});await wait(80);
   const beforeEnd=await page.evaluate(()=>__hookechoMapTrace.frames);
   await inputSession.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});await wait(300);
   report.pinch={reactedBeforeRelease:beforeEnd.some(f=>Math.abs(f[9]-1)>0.01&&Math.abs(f[3]-beforeEnd[0][3])>0.01)};
   assert(report.pinch.reactedBeforeRelease,'touch pinch must update during the gesture');
  }
  await page.close();
 }
 if(process.env.EXPECT_MOTION==='1')assert(report.drag?.movedAfterRelease,'drag release must coast');
 assert.deepEqual(report.errors,[]);await fs.writeFile(out+'/report.json',JSON.stringify(report,null,2));
}finally{await browser.close()}
