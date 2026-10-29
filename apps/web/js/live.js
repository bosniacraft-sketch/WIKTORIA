// THE LINE TO THE GAME. Everything the page knows comes through here (see API.md).
// S is the latest answer to "what does the screen show"; every draw function reads S and nothing else.
let S = null;              // the whole screen, as the game last described it
let selected = null;       // the province the player clicked, or null
let online = false;
let RESEARCH = null;       // GET /api/research while the Technology window is open, else null
let armySel = null;        // the id of the army picked in the outliner to be moved, or null
let notice = '';           // the answer to the last order, shown under the country's name
let EVENTS = null;         // GET /api/events while something waits or the Politics window is open
let DIPLO = null;          // GET /api/diplomacy while a question waits or the Diplomacy window is open
let POLITICS = null;       // GET /api/politics while the Politics window is open (js/windows/politics.js)

async function ask(path, method = 'GET') {
  const r = await fetch(path, { method, cache: 'no-store' });
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return r.json();
}
const tell = path => ask(path, 'POST').catch(() => null);

// An order of the player (see API.md, Orders): its answer, or its refusal, is shown as `notice`.
async function order(path) {
  try {
    const r = await fetch(path, { method: 'POST', cache: 'no-store' });
    const a = await r.json().catch(() => ({}));
    notice = r.ok ? esc(a.text || '') : `<span class="bad">${esc(a.error || 'refused')}</span>`;
  } catch (e) { notice = '<span class="bad">The game is not answering.</span>'; }
  await refresh();
  await refreshPlay(true); drawAll();
}

let researchAsked = 0;
async function refreshResearch(force = false) {
  if (open !== 'technology') { RESEARCH = null; return; }
  if (!force && Date.now() - researchAsked < 2000) return;
  researchAsked = Date.now();
  try { RESEARCH = await ask('/api/research'); } catch (e) { /* the next round tries again */ }
}

// What waits for a played seat (API.md: Events and decisions, Diplomacy): asked only when the
// screen says something waits, or its window is open.
const playAsked = { events: 0, diplomacy: 0 };
async function refreshPlay(force = false) {
  const w = S?.country?.waiting || {}, now = Date.now();
  const want = { events: w.events > 0 || open === 'politics', diplomacy: w.questions > 0 || open === 'diplomacy' };
  try {
    if (!want.events) EVENTS = null;
    else if (force || now - playAsked.events > 1000) { playAsked.events = now; EVENTS = await ask('/api/events'); }
    if (!want.diplomacy) DIPLO = null;
    else if (force || now - playAsked.diplomacy > 1500) { playAsked.diplomacy = now; DIPLO = await ask('/api/diplomacy'); }
  } catch (e) { /* the next round tries again */ }
}

let politicsAsked = 0;
async function refreshPolitics(force = false) {
  if (open !== 'politics') { POLITICS = null; return; }
  if (!force && Date.now() - politicsAsked < 2000) return;
  politicsAsked = Date.now();
  try { POLITICS = await ask('/api/politics'); } catch (e) { /* the next round tries again */ }
}

async function refresh() {
  try {
    S = await ask(`/api/screen?province=${selected ?? ''}`);
    online = true;
    if (!S.country?.played) armySel = null;
    await refreshResearch();
    await refreshPlay();
    if (typeof refreshPopulation === 'function') await refreshPopulation();   // js/windows/population.js
    await refreshPolitics();
    if (typeof refreshMilitary === 'function') await refreshMilitary();   // js/windows/military.js
  } catch (e) { online = false; }
  drawAll();
}

// The map picture: fetched again when the game says owners changed, or the mode changes.
// Layers: our own map look, composed by the game; `tag` is the country that glows (the seat).
const MAP = { mode: 'political', version: -1, modes: ['political'], width: 4096,
  layers: ['terrain', 'owners', 'rivers', 'borders', 'glow'], shown: new Set(['terrain', 'owners', 'rivers', 'borders', 'glow']), tag: '' };
async function refreshMap(force = false) {
  try {
    const m = await ask('/api/map');
    MAP.modes = m.modes || MAP.modes;
    MAP.layers = m.layers || MAP.layers;
    MAP.full_width = m.width || MAP.full_width;
    MAP.full_height = m.height || MAP.full_height;
    if (force || m.version !== MAP.version) {
      MAP.version = m.version;
      const layers = MAP.layers.filter(l => MAP.shown.has(l)).join(',');
      $('mapimg').src = `/map.bmp?mode=${encodeURIComponent(MAP.mode)}&width=${MAP.width}&layers=${layers}&tag=${encodeURIComponent(MAP.tag)}&v=${m.version}`;
      drawMini();
    }
    if (typeof refreshLabels === 'function') refreshLabels(m.version);
  } catch (e) { /* the game is not answering; the next round tries again */ }
}
