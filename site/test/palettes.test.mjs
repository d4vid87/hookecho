import test from 'node:test';
import assert from 'node:assert/strict';
import { palettes, paletteVariables, DEFAULT_PALETTE } from '../src/data/palettes.js';

const luminance = hex => {
  const rgb = hex.slice(1).match(/../g).map(c => parseInt(c,16)/255).map(c => c <= .04045 ? c/12.92 : ((c+.055)/1.055)**2.4);
  return rgb[0]*.2126 + rgb[1]*.7152 + rgb[2]*.0722;
};
const contrast = (a,b) => (Math.max(luminance(a),luminance(b))+.05)/(Math.min(luminance(a),luminance(b))+.05);

test('ten distinct palettes keep interface text readable on their surfaces', () => {
  assert.equal(palettes.length,10);
  assert.equal(DEFAULT_PALETTE,'carbon');
  assert.equal(new Set(palettes.map(p=>p.id)).size,10);
  assert.equal(palettes.filter(p=>p.mode === 'light').length,5);
  for (const p of palettes) {
    const c = p.colors;
    for(const value of Object.values(c)) assert.match(value,/^#[0-9a-f]{6}$/i);
    for(const [fg,bg] of [[c.text,c.bg],[c.text,c.panel],[c.muted,c.bg],[c.muted,c.panel],[c.accent,c.bg],[c.accent,c.panel],[c.actionInk,c.action],['#c3ceca',c.radar]]) {
      assert.ok(contrast(fg,bg)>=4.5,`${p.name}: ${fg} on ${bg} has contrast ${contrast(fg,bg).toFixed(2)}`);
    }
    assert.ok(paletteVariables(p).includes(`--bg:${c.bg}`));
    assert.ok(paletteVariables(p).includes(`color-scheme:${p.mode}`));
  }
});
