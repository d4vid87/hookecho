import assert from 'node:assert/strict';

// Read-only geometry from egui's named controls; actions use the browser's real input events.
export async function installUiTargets(page) {
  await page.evaluateOnNewDocument(() => {
    globalThis.__hookechoUiTargets = {pass: -1, controls: {}};
    globalThis.__hookechoUiTarget = (name, pass, x, y, w, h, scale) => {
      const targets = globalThis.__hookechoUiTargets;
      if (targets.pass !== pass) { targets.pass = pass; targets.controls = {}; }
      if (w > 0 && h > 0) (targets.controls[name] ??= []).push({x, y, w, h, scale});
    };
  });
}

export async function targetPoint(page, name) {
  try {
    await page.waitForFunction(name => globalThis.__hookechoUiTargets?.controls[name]?.length, {timeout: 15000}, name);
  } catch (error) {
    const available = await page.evaluate(() => Object.keys(globalThis.__hookechoUiTargets?.controls ?? {}));
    throw new Error(`Control ${name} not found; visible: ${available.join(', ')}`, {cause: error});
  }
  const targets = await page.evaluate(name => {
    const canvas = document.getElementById('hookecho');
    const rect = canvas.getBoundingClientRect();
    return globalThis.__hookechoUiTargets.controls[name].map(({x, y, w, h, scale}) => ({
      x: rect.left + (x + w / 2) * scale * rect.width / canvas.width,
      y: rect.top + (y + h / 2) * scale * rect.height / canvas.height,
    }));
  }, name);
  assert.equal(targets.length, 1, `expected one visible control named ${name}`);
  return targets[0];
}
export async function clickTarget(page, name) {
  const {x, y} = await targetPoint(page, name);
  await page.mouse.click(x, y);
  await new Promise(resolve => setTimeout(resolve, 350));
}
