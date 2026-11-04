// Local visual regression: requires Chromium, Puppeteer, and Tesseract OCR (TESSERACT_BIN).
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {fixtureRadar} from './fixtures/radar.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8816/';
const output = process.env.HOOKECHO_CHASER_OUTPUT ?? 'outputs/chaser-dashboard';
await fs.mkdir(output, {recursive: true});
const marker = index => index===0 ? 'FIRST' : index===99 ? 'LAST' : String(index);
const longName = index => `${String(index).padStart(3, '0')} Storm Chaser With A Very Long Full Public Display Name NAME_END_${marker(index)}`;
const longStatus = index => `Reporting a very long road condition and movement status that must remain readable in the detailed view STATUS_END_${marker(index)}`;
const stream = 'https://www.youtube.com/@LongPublicStormChasingChannelForDashboardRegression/live';
const at = new Date().toISOString().slice(0,19).replace('T', ' ');
const fixture = Array.from({length:100}, (_, index) => `Object: ${41.73 + index * .0001},${-93.72 + index * .0001}\nIcon: 0,0,0,6,10,"${longName(index)}\\n${at} UTC\\n${longStatus(index)}\\nWeb: ${stream}"\nEnd:`).join('\n');
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const report = {checks:[], errors:[]};
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
let page;
const execute = promisify(execFile);
// Canvas text is checked through OCR so truncated rows cannot hide lost report data.
async function capture(name) {
  const path = `${output}/${name}.png`;
  await page.screenshot({path});
  const {stdout} = await execute(process.env.TESSERACT_BIN ?? 'tesseract', [path, 'stdout', 'tsv']);
  const words = stdout.split('\n').slice(1).map(line => line.split('\t')).filter(parts => parts.length >= 12 && parts[11].trim())
    .map(parts => ({text:parts[11],x:+parts[6],y:+parts[7],width:+parts[8],height:+parts[9]}));
  const text = words.map(word => word.text).join(' ');
  await fs.writeFile(`${output}/${name}.txt`, text);
  return {text,words};
}
function contains(capture, text) {
  const normalize = value => value.replace(/[^a-z0-9]/gi,'').toLowerCase();
  assert(normalize(capture.text).includes(normalize(text)), `expected visible text ${JSON.stringify(text)} in ${capture.text}`);
}
async function hover(x,y) { await page.mouse.move(x,y); await wait(1000); }
async function clickWord(word) { assert(word,'expected visible action'); await page.mouse.click(word.x+word.width/2,word.y+word.height/2); await wait(500); }
try {
  page = await browser.newPage();
  await page.setViewport({width:1280,height:900});
  await page.setBypassServiceWorker(true);
  const requests = await fixtureRadar(page, {loopFrames:true,vector:true,spottersResponse:async()=>({status:200,contentType:'text/plain',body:fixture}),
    vectorResponse: async()=>({status:200,contentType:'application/json',body:JSON.stringify({tiles:[base+'vector-fixture/{z}/{x}/{y}.pbf']})})});
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator,'gpu',{value:undefined});
    window.open = url => { globalThis.__openedStream = url; return null; };
    if (!localStorage.getItem('hookecho-settings')) localStorage.setItem('hookecho-settings',JSON.stringify({
      setup_done:true,seeded_workspaces:true,default_site:'KDMX',radar_feed:'ArchiveOnly',spotter_range_km:0,
    }));
  });
  await page.goto(base+'#goto=KDMX,-93.72,41.73,8.5', {waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>!document.getElementById('boot'),{timeout:90000});
  async function openDashboard() {
    await page.keyboard.down('Control'); await page.keyboard.press('k'); await page.keyboard.up('Control');
    await wait(250);
    await page.keyboard.type('Storm chasers'); await page.keyboard.press('Enter');
    await wait(1000);
  }
  await openDashboard();
  for(let i=0;i<40&&!requests.spotterRequests.length;i++) await wait(250);
  assert(requests.spotterRequests.length>0,'dashboard must load the Spotter Network feed');
  await wait(1500);
  const dismiss = await page.$('#dismiss-install');
  if (dismiss && await dismiss.isVisible()) await dismiss.click();
  await page.mouse.click(702,202); // Close the command palette behind the dashboard.
  const initial = await capture('desktop');
  contains(initial,'100 reported positions');
  await hover(190,234);
  contains(await capture('desktop-name-hover'),'NAME_END_FIRST');
  await hover(190,275);
  contains(await capture('desktop-status-hover'),'STATUS_END_FIRST');
  await page.mouse.move(700,500); await wait(500);
  await page.mouse.click(230,309);
  await wait(500);
  const openedStream = await page.evaluate(()=>globalThis.__openedStream);
  assert.equal(openedStream,stream,'Open stream must retain the published URL');
  report.checks.push('Desktop: 100 entries, full name/status on hover, original stream URL opens');
  await page.mouse.move(250,600); await page.mouse.wheel({deltaY:30000}); await wait(300); await page.mouse.move(800,500); await wait(500);
  const end = await capture('desktop-end');
  contains(end,'099');
  await clickWord(end.words.filter(word=>word.text.startsWith('View') && word.x<200).at(-1));
  await wait(1000);
  const detail = await capture('desktop-detail');
  contains(detail,'NAME_END_LAST');
  contains(detail,'STATUS_END_LAST');
  const pane = await page.evaluate(()=>{const scene=JSON.parse(atob(location.hash.slice(7).replaceAll('-','+').replaceAll('_','/'))); return scene.workspace.panes[scene.workspace.active];});
  assert(Math.abs(pane.lon-(-93.7101))<.00001 && Math.abs(pane.lat-41.7399)<.00001, 'View on map must center the last entry');
  report.checks.push('Desktop: last virtualized entry remains reachable, View on map centers it, detail keeps full name/status');
  await page.setViewport({width:390,height:844}); await wait(500);
  await capture('narrow-detail-resize');
  await page.setViewport({width:1280,height:900}); await wait(500);
  await clickWord(detail.words.find(word=>word.text.startsWith('Close')));
  await openDashboard();
  await page.mouse.click(702,202);
  await page.setViewport({width:390,height:844}); await wait(500);
  await page.mouse.move(200,500); await page.mouse.wheel({deltaY:-30000}); await wait(300); await page.mouse.move(389,840); await wait(500);
  const narrow = await capture('narrow');
  contains(narrow,'100 reported positions');
  contains(narrow,'000');
  const name = narrow.words.find(word=>word.text==='000');
  await hover(name.x+50,name.y+name.height/2);
  const narrowName = await capture('narrow-name-hover');
  contains(narrowName,'NAME_END_FIRST');
  await page.mouse.move(389,840); await wait(500);
  const status = narrow.words.find(word=>word.text==='Reporting');
  await hover(status.x+30,status.y+status.height/2);
  contains(await capture('narrow-status-hover'),'STATUS_END_FIRST');
  await page.mouse.move(200,500); await page.mouse.wheel({deltaY:30000}); await wait(300); await page.mouse.move(389,840); await wait(500);
  const narrowEnd = await capture('narrow-end');
  contains(narrowEnd,'099');
  await clickWord(narrowEnd.words.filter(word=>word.text.startsWith('View') && word.x<200).at(-1));
  await wait(500);
  const narrowDetail = await capture('narrow-detail');
  contains(narrowDetail,'099 Storm Chaser');
  assert(!narrowDetail.text.includes('100 reported positions'),'View on map must close the dashboard');
  const narrowPane = await page.evaluate(()=>{const scene=JSON.parse(atob(location.hash.slice(7).replaceAll('-','+').replaceAll('_','/'))); return scene.workspace.panes[scene.workspace.active];});
  assert(Math.abs(narrowPane.lon-(-93.7101))<.00001 && Math.abs(narrowPane.lat-41.7399)<.00001, 'Narrow View on map must select the last entry');
  report.checks.push('390px panel: full name/status on hover, last entry reachable and View on map retains its position');
  report.existingLimitations = [
    'The shared Detail window can clip long text when narrowed to phone width; complete report text remains accessible in the dashboard tooltips and desktop Detail.',
    'At phone width, existing mobile station/product controls overlap the dashboard header and bottom navigation overlaps its footer; the final row map action remains reachable.',
  ];
  assert.deepEqual(report.errors,[],'dashboard interaction must not throw browser errors');
  console.log(report.checks.join('\n'));

} catch(error) {
  report.errors.push(error.stack??String(error));
  if(page) await page.screenshot({path:output+'/failure.png'}).catch(()=>{});
  throw error;
} finally {
  await fs.writeFile(output+'/report.json',JSON.stringify(report,null,2));
  await browser.close();
}
