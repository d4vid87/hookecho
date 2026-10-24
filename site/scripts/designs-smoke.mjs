// npm run dev first. Pass --capture to refresh the clearly labelled sample gallery images.
import {chromium} from 'playwright';
import assert from 'node:assert/strict';
import {mkdir,readFile} from 'node:fs/promises';
import {websiteDesigns} from '../src/data/website-designs.js';
const base = process.env.DESIGN_BASE_URL || 'http://127.0.0.1:4347';
const capture = process.argv.includes('--capture');
const browser = await chromium.launch({headless:true, ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH ? {executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH} : {})});
const output = new URL('../public/design-studies/', import.meta.url);
const radarImage = `data:image/jpeg;base64,${(await readFile(new URL('../public/shots/mrms.jpg',import.meta.url))).toString('base64')}`;
const now = Date.parse('2026-10-23T12:00:00-05:00');
const conditions = ['Mostly Sunny','Partly Cloudy','Sunny','Mostly Clear','Chance Rain Showers','Showers And Thunderstorms','Partly Sunny'];
const hourly=Array.from({length:24},(_,i)=>({name:i?'Friday':'This Afternoon',startTime:new Date(now+i*3600000).toISOString(),endTime:new Date(now+(i+1)*3600000).toISOString(),temperature:[79,80,81,80,78,76,73,71,69,67,65,63][i%12],temperatureUnit:'F',isDaytime:i<7||i>18,shortForecast:conditions[Math.floor(i/4)%7],detailedForecast:'Sunny with a light south wind. Check the latest forecast before heading out.',windSpeed:'8 mph',windDirection:'S',relativeHumidity:{value:42},probabilityOfPrecipitation:{value:i>16?25:5}}));
const daily=Array.from({length:14},(_,i)=>({...hourly[0],name:i===0?'Today':i===1?'Tonight':`${['Saturday','Sunday','Monday','Tuesday','Wednesday','Thursday'][Math.floor((i-2)/2)]}${i%2?' Night':''}`,startTime:new Date(now+(i*12-6)*3600000).toISOString(),endTime:new Date(now+(i*12+6)*3600000).toISOString(),temperature:i%2?[58,59,57,61,60,58,56][Math.floor(i/2)]:[81,82,79,76,74,77,80][i/2],isDaytime:i%2===0,shortForecast:conditions[i%7],probabilityOfPrecipitation:{value:[5,0,10,5,25,40,60,45,20,10,5,0,5,10][i]}}));
try {
 if(capture) await mkdir(output,{recursive:true});
 for(const d of websiteDesigns){
  const page=await browser.newPage({viewport:{width:1440,height:1080},reducedMotion:'reduce'});
  const errors=[]; page.on('pageerror',e=>errors.push(e.message));
  await page.clock.setFixedTime(new Date(now));
  await page.route('https://app.hookecho.io/**',r=>r.fulfill({contentType:'text/html',body:`<html><head><meta charset="utf-8"/></head><body style="margin:0;background:#0d171e;height:100vh"><img alt="Recorded HookEcho radar frame for design comparison" src="${radarImage}" style="width:100%;height:100%;object-fit:cover"/><span style="position:absolute;bottom:16px;left:16px;padding:9px 12px;background:#0c1825;color:white;font:12px sans-serif">RECORDED RADAR · DESIGN PREVIEW</span></body></html>`}));
  await page.route('https://api.weather.gov/**',r=>{const url=r.request().url();return r.fulfill({json:url.includes('/points/')?{properties:{forecastHourly:'https://api.weather.gov/test/hourly',forecast:'https://api.weather.gov/test/daily',timeZone:'America/Chicago'}}:url.includes('/alerts/')?{features:[]}:{properties:{periods:url.endsWith('/daily')?daily:hourly,updateTime:new Date(now).toISOString()}}});});
  await page.goto(`${base}/designs/${d.id}/`);
  await page.locator('.day-period').last().waitFor();
  await page.evaluate(()=>document.fonts.ready);
  await page.frameLocator('[data-live-radar]').locator('img').evaluate(image=>image.decode());
  assert.equal(await page.locator('.hour').count(),24,d.id);
  assert.equal(await page.locator('.day-entry').count(),7,d.id);
  assert.equal(await page.locator('.day-period').count(),14,d.id);
  assert.equal(await page.locator('.period-date').first().textContent(),'Oct 23');
  assert.equal(await page.locator('[data-live-radar]').count(),1);
  assert.equal(await page.locator('[data-feature]').count(),3);
  if(capture) await page.screenshot({path:new URL(`${d.id}-overview.jpg`,output).pathname,type:'jpeg',quality:82});
  await page.locator('#week').evaluate(e=>window.scrollTo(0,e.getBoundingClientRect().top+scrollY-20));
  if(capture) await page.screenshot({path:new URL(`${d.id}-week.jpg`,output).pathname,type:'jpeg',quality:82});
  await page.locator('.day-entry summary').first().focus();await page.keyboard.press('Enter');
  assert.equal(await page.locator('.day-entry').first().getAttribute('open'),'');
  await page.locator('[data-unit=C]').click();
  assert.equal(await page.locator('[data-temperature]').textContent(),'26°');
  await page.locator('[data-unit=F]').click();
  if(['mega','drawer'].includes(d.menu)){
   await page.locator('.design-expand summary').click();
   assert.ok(await page.locator('.design-expand nav').isVisible());
   await page.keyboard.press('Escape');
   assert.ok(!(await page.locator('.design-expand nav').isVisible()));
  }
  for(const width of [768,390,320]){
   await page.setViewportSize({width,height:900});
   assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`${d.id}: horizontal overflow at ${width}`);
   assert.ok(await page.locator('.period-rain').first().isVisible());
   const boxes=await page.locator('.day-period').evaluateAll(es=>es.map(e=>({parent:e.getBoundingClientRect().toJSON(),children:[...e.children].map(c=>c.getBoundingClientRect().toJSON())})));
   assert.ok(boxes.every(b=>b.children.every(c=>c.right<=b.parent.right+2)),`${d.id}: forecast content overflows at ${width}`);
   if(capture&&width===390){await page.locator('#week').scrollIntoViewIfNeeded();await page.screenshot({path:`/tmp/design-${d.id}-mobile.png`});}
  }
  assert.deepEqual(errors,[],d.id);
  await page.close();console.log(`PASS ${d.name}: forecast, units, keyboard details, navigation, desktop/tablet/mobile`);
 }
 const gallery=await browser.newPage({viewport:{width:390,height:900}});
 await gallery.goto(`${base}/designs/`);
 assert.equal(await gallery.locator('[data-design-image]').count(),10);
 await gallery.getByRole('button',{name:'Weekly forecast',exact:true}).click();
 assert.ok((await gallery.locator('[data-design-image]').first().getAttribute('src')).endsWith('-week.jpg'));
 assert.ok(await gallery.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
 console.log('PASS gallery: ten designs, view switch, mobile layout');
} finally {await browser.close();}
