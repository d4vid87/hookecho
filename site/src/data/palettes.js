// Interface colors only. Radar imagery and warning severity colors remain unchanged.
export const DEFAULT_PALETTE = 'carbon';
const entries = [
  ['cobalt', 'Cobalt', 'light', 'Cool paper. Electric blue. Precise and technical.', '#eef2f8', '#ffffff', '#dde5f0', '#10213b', '#4d5f78', '#b7c5d8', '#164bc0', '#245eea', '#ffffff', '#101e35'],
  ['alpine', 'Alpine', 'light', 'Soft ivory and forest green. Calm, grounded, outdoors.', '#eff1e7', '#fafcf3', '#dfe5d5', '#19291d', '#52634c', '#b5c1aa', '#24603c', '#286341', '#ffffff', '#17291e'],
  ['copper', 'Copper', 'light', 'Warm parchment with burnt copper. An editorial field manual.', '#f4ece1', '#fff8ee', '#e8dac7', '#33251e', '#705b4e', '#cbbbA5', '#9b3c1c', '#ac4320', '#ffffff', '#30211b'],
  ['lagoon', 'Lagoon', 'light', 'Pale mineral green with deep teal. Clear and refreshing.', '#e8f2ef', '#f7fffc', '#d2e6e0', '#14332e', '#47685f', '#a8c6bd', '#006a61', '#006d63', '#ffffff', '#12352f'],
  ['orchid', 'Orchid', 'light', 'Lavender paper and rich violet. A softer technical character.', '#f0edf7', '#fcfaff', '#e2dced', '#30223f', '#655570', '#c0b5ce', '#6b32a5', '#7239b2', '#ffffff', '#2a1d3a'],
  ['midnight', 'Midnight', 'dark', 'Deep navy with icy blue. A focused overnight workstation.', '#0c1728', '#14243a', '#1c3049', '#edf4ff', '#a9bbd3', '#3d5470', '#83c5ff', '#83c5ff', '#0c2037', '#08111f'],
  ['carbon', 'Carbon', 'dark', 'Charcoal and amber. Instrument-panel contrast.', '#1b1b1a', '#252523', '#30302c', '#f4f1e7', '#c0bcae', '#57564d', '#ffc45d', '#ffc45d', '#302007', '#121211'],
  ['aurora', 'Aurora', 'dark', 'Deep evergreen and acid lime. Sharp, energetic, field-ready.', '#101e19', '#1b2c24', '#25392e', '#eef6e9', '#afc4b4', '#496451', '#c0ed72', '#c0ed72', '#192509', '#0a1510'],
  ['merlot', 'Merlot', 'dark', 'Black cherry and dusty rose. Atmospheric without the glow.', '#24161e', '#32212b', '#432d39', '#fff0f6', '#d1b0c0', '#715060', '#ffa5c5', '#ffa5c5', '#391528', '#180e14'],
  ['monochrome', 'Monochrome', 'dark', 'Near-black and chalk white. Pure hierarchy, minimal color.', '#151515', '#222222', '#303030', '#f6f6f2', '#bdbdb8', '#555552', '#eeeeea', '#eeeeea', '#181818', '#090909'],
];

export const palettes = entries.map(([id, name, mode, description, bg, panel, secondary, text, muted, line, accent, action, actionInk, radar]) => ({
  id, name, mode, description,
  colors: {bg, panel, secondary, text, muted, line, accent, action, actionInk, radar},
}));

export function paletteVariables(palette) {
  const c = palette.colors;
  return `--bg:${c.bg};--bg-deep:${c.bg};--surface-1:${c.panel};--surface-2:${c.secondary};--text:${c.text};--text-dim:${c.muted};--stroke:${c.line};--stroke-strong:${c.line};--accent:${c.accent};--accent-ink:${c.actionInk};--palette-action:${c.action};--palette-action-ink:${c.actionInk};--palette-dark:${c.radar};--palette-light:#f3f5f4;--palette-dim:#c3ceca;--ramp-1:${c.accent};--ramp-3:${c.accent};--ramp:linear-gradient(${c.action},${c.action});color-scheme:${palette.mode}`;
}

export const paletteStyles = palettes.map(p => `html.site-signal[data-palette="${p.id}"]{${paletteVariables(p)}}`).join('\n');
