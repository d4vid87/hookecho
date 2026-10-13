import fs from 'node:fs/promises';
export async function fixtureRadar(page, options = {}) {
 const fixture = await fs.readFile(new URL('../../../crates/wxdata/tests/data/kdmx-one-sweep.bin',import.meta.url));
 const abi = options.abi ? await fs.readFile(new URL("../../../crates/wxdata/tests/data/g19-c13-meso.nc", import.meta.url)) : null;
 const l3 = await fs.readFile(new URL('../../../crates/nexrad-level3/tests/data/n0g_tlx.l3',import.meta.url));
 const counts = {aborted:[],radarRequests:[],radarResponses:[],list:0,volume:0,abiRequests:[],vectorRequests:[],satelliteCatalogs:0,satelliteTimes:[],satelliteMethods:[]};
 const tile = options.satellite ? await fs.readFile(new URL("../../../web/icon-192.png", import.meta.url)) : null;
 await page.setRequestInterception(true);
 page.on('request',async request=>{
  const url = new URL(request.url());
  if (url.href.includes("nexrad-level2.s3.amazonaws.com")) counts.radarRequests.push(url.href);
  if(options.networkState?.offline && (url.pathname.startsWith("/proxy/") || !["127.0.0.1", "localhost"].includes(url.hostname))) { counts.aborted.push(url.href); await request.abort("internetdisconnected"); return; }
  if(options.vector && (url.href.includes('tiles.openfreemap.org') || url.pathname.startsWith('/vector-fixture/'))) {
   counts.vectorRequests.push(url.href);
   if(url.href.includes('tiles.openfreemap.org')) await request.respond({status:200,contentType:'application/json',body:JSON.stringify({tiles:['http://127.0.0.1:8765/vector-fixture/{z}/{x}/{y}.pbf']})});
   else await request.respond({status:200,contentType:'application/x-protobuf',body:Buffer.alloc(0)});
  } else if(options.abi && url.href.includes('noaa-goes')) {
   counts.abiRequests.push(url.href);
   if(url.searchParams.has('list-type')) {
    const prefix=url.searchParams.get('prefix');
    const key=`${prefix}_G19_s20251001800310_e20251001800410_c20251001800510.nc`;
    await request.respond({status:200,contentType:'application/xml',body:`<ListBucketResult><Key>${key}</Key></ListBucketResult>`});
   } else await request.respond({status:200,contentType:'application/octet-stream',body:abi});
  } else if(options.satellite && url.href.includes('gibs.earthdata.nasa.gov')) {
   counts.satelliteMethods.push([request.method(),url.href,request.headers()]);
   if(url.pathname.endsWith('.xml')) {
    counts.satelliteCatalogs++;
    await request.respond({status:200,contentType:'application/xml',headers:{'Access-Control-Allow-Origin':'*','Access-Control-Allow-Headers':'*','Access-Control-Allow-Methods':'GET,OPTIONS','Access-Control-Allow-Private-Network':'true'},body:'<Domains><Domain>2026-10-12T17:00:00Z/2026-10-12T17:20:00Z/PT10M</Domain></Domains>'});
   } else {
    counts.satelliteTimes.push(url.href);
    await request.respond({status:200,contentType:'image/png',headers:{'Access-Control-Allow-Origin':'*','Access-Control-Allow-Headers':'*','Access-Control-Allow-Methods':'GET,OPTIONS','Access-Control-Allow-Private-Network':'true'},body:tile});
   }
  } else if(url.href.includes('nexrad-level3.s3.amazonaws.com')) {
   if(url.searchParams.has('list-type')) {
    counts.list++;
    const prefix=url.searchParams.get('prefix');
    await request.respond({status:200,contentType:'application/xml',body:`<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>${prefix}_17_00_00</Key></Contents></ListBucketResult>`});
   } else { counts.volume++; await request.respond({status:200,contentType:'application/octet-stream',body:l3}); }
  } else if(url.href.includes('nexrad-level2.s3.amazonaws.com')) {
   if(url.searchParams.has('list-type')) {
    counts.list++;
    const prefix=url.searchParams.get('prefix');
    const parts=prefix.split('/');
    const times=options.loopFrames ? ['170000','170100','170200','170300','170400','170500'] : ['170000'];
    const site=options.loopFrames ? parts[3] : 'KDMX';
    const contents=times.map(time=>`<Contents><Key>${prefix}/${site}${parts.slice(0,3).join('')}_${time}_V06</Key><LastModified>${parts.slice(0,3).join('-')}T17:00:00Z</LastModified><Size>${fixture.length}</Size></Contents>`).join('');
    await request.respond({status:200,contentType:'application/xml',body:`<ListBucketResult><IsTruncated>false</IsTruncated>${contents}</ListBucketResult>`});
    counts.radarResponses.push(url.href);
   } else {
    counts.volume++;
    await request.respond({status:200,contentType:'application/octet-stream',body:fixture});
    counts.radarResponses.push(url.href);
   }
  } else await request.continue();
 });
 return counts;
}
