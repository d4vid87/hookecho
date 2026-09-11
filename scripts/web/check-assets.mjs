// Check production aliases too: a healthy deployment URL can hide a poisoned CDN response.
for (const origin of process.argv.slice(2)) {
  const page = await fetch(origin);
  if (!page.ok) throw new Error(`${origin}: HTTP ${page.status}`);
  const html = await page.text();
  const paths = [...html.matchAll(/(?:href=")(\.\/dist\/hookecho[^" ]+\.(?:js|wasm)(?:\?[^" ]*)?)/g)].map(m => m[1]);
  if (paths.length !== 2) throw new Error(`${origin}: missing bundle references`);
  for (const path of paths) {
    const response = await fetch(new URL(path, origin));
    const expected = path.includes(".wasm") ? "application/wasm" : "javascript";
    if (!response.ok || !response.headers.get("content-type")?.includes(expected)) {
      throw new Error(`${origin} ${path}: ${response.status} ${response.headers.get("content-type")}`);
    }
    await response.body.cancel();
  }
  console.log(`${origin}: executable bundle assets OK`);
}
