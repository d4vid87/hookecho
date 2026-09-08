// Run from the repository root with Playwright and Chromium installed.
// Captures the browser layout;
// Native Android captures use shoot.sh android.
const{chromium}=require('playwright');
const fs=require('node:fs');
(async()=>{const b=await chromium.launch({executablePath:'/usr/bin/chromium',args:['--no-sandbox']});
const p=await b.newPage({viewport:{width:430,height:932}});
fs.mkdirSync('docs/shots/mobile',{recursive:true});
await p.goto('https://app.hookecho.io/#goto=KBMX,-87.55,33.20,8.3,2011-04-27T22:10:00Z,bm:dark',{waitUntil:'domcontentloaded'});
await p.waitForTimeout(18000);
await p.screenshot({path:'docs/shots/mobile/map.jpg',type:'jpeg',quality:85});
await p.mouse.click(336,68);
await p.mouse.move(425,920);
await p.waitForTimeout(1500);
await p.screenshot({path:'docs/shots/mobile/layers.jpg',type:'jpeg',quality:85});
await p.mouse.click(265,33);
await p.mouse.move(425,920);
await p.waitForTimeout(1500);
await p.screenshot({path:'docs/shots/mobile/alerts.jpg',type:'jpeg',quality:85});
await p.keyboard.press('Escape');
await p.goto('https://app.hookecho.io/#goto=KTLX,-97.36,35.40,8.9,2013-05-20T20:15:00Z,bm:dark');
await p.reload();
await p.waitForTimeout(20000);
fs.mkdirSync('/tmp/hookecho-mobile-frames',{recursive:true});
for(let i=0;
i<8;
i++){await p.screenshot({path:`/tmp/hookecho-mobile-frames/${i}.png`});
await p.keyboard.press('ArrowRight');
await p.waitForTimeout(3000);
}await b.close()})();

