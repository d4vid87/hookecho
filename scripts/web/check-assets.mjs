import {shellAssets} from './shell-assets.mjs';

// Check production aliases too: a healthy deployment URL can hide a poisoned CDN response.
for (const origin of process.argv.slice(2)) {
  const page = await fetch(origin);
  if (!page.ok) throw new Error(`${origin}: HTTP ${page.status}`);
  const html = await page.text();
  const paths = await shellAssets(html, async path => {
    const response = await fetch(new URL(path, origin));
    if (!response.ok || !response.headers.get('content-type')?.includes('javascript')) {
      throw new Error(`${origin} ${path}: ${response.status} ${response.headers.get('content-type')}`);
    }
    return response.text();
  });
  // HLS is optional and loads on demand; verify its deployment without precaching it.
  paths.push('/vendor/hls-1.7.3.min.js');
  for (const path of paths.filter(path => path !== '/')) {
    const response = await fetch(new URL(path, origin));
    const type = response.headers.get('content-type') || '';
    const expected = path.includes('.wasm') ? 'application/wasm' : path.includes('.js') ? 'javascript' : null;
    if (!response.ok || (expected ? !type.includes(expected) : type.includes('text/html'))) {
      throw new Error(`${origin} ${path}: ${response.status} ${type}`);
    }
    await response.body.cancel();
  }
  console.log(`${origin}: complete app shell assets OK`);
}
