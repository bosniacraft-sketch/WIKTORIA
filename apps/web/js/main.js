// WIRING: the start, clicks and keys, moving the map, and the regular refresh.

// ---- the start: what the game was started with, and whose seat to take
let seat = null;
async function start() {
  try {
    const [install, countries] = await Promise.all([ask('/api/install'), ask('/api/countries')]);
    Boot.ready({ install, countries });
    $('start-band').textContent = 'Welcome.';
    const s = $('install-status');
    s.className = 'status ' + (install.ok ? 'ok' : 'bad');
    s.textContent = install.ok ? `✓ Found Victoria 2${install.version ? ', version ' + install.version : ''}: ${install.path}` + ((install.mods || []).length ? ` · mods: ${install.mods.join(', ')}` : '') : `Victoria 2 was not found at ${install.path}.`;
    const land = countries.filter(c => c.exists);
    $('seats').innerHTML = land.slice(0, 16).map(c => `<button class="seat" data-pick-seat="${esc(c.tag)}"><b>${c.rank ?? ''}</b> ${esc(c.name)}</button>`).join('')
      + `<label class="more">or any other: <select id="seat-all"><option value="">&hellip;</option>${land.slice(16).map(c => `<option value="${esc(c.tag)}">${esc(c.name)}</option>`).join('')}</select></label>`;
    $('seat-all').onchange = e => chooseSeat(e.target.value);
  } catch (e) {
    // The game may still be loading: say so and try again by itself.
    $('start-band').textContent = 'Waiting for the game to start…';
    setTimeout(start, 1000);
  }
}
function chooseSeat(tag) {
  if (!tag) return; seat = tag;
  document.querySelectorAll('.seat').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.pickSeat === tag)));
  $('play').disabled = false; $('observe').disabled = false;
}
// Play (take control of the chosen country) or Observe (the computer plays everyone): D20.
async function play(control = true) {
  await tell(`/api/seat?tag=${encodeURIComponent(seat)}`);
  await tell(control ? `/api/play?tag=${encodeURIComponent(seat)}` : '/api/observe');
  MAP.tag = seat;
  $('start').hidden = true;
  Boot.mounting((document.querySelector(`.seat[data-pick-seat="${seat}"]`) || {}).textContent || seat);
  await refreshMap(true); await refresh();
  Boot.mounted(S && S.province === undefined ? '' : 'MOUNTED');
  if (!ticking) { ticking = true; setInterval(refresh, 500); setInterval(refreshMap, 3000); }
}
let ticking = false;   // the regular refresh, started once even when the start screen is shown again

// ---- clicks
document.addEventListener('click', async e => {
  const t = e.target, on = sel => t.closest(sel);
  let b;
  if ((b = on('[data-pick-seat]'))) return chooseSeat(b.dataset.pickSeat);
  if (on('#play')) return play(true);
  if (on('#observe')) return play(false);
  if ((b = on('[data-win]'))) { open = open === b.dataset.win ? null : b.dataset.win; await refreshResearch(true); await refreshPlay(true); return drawAll(); }
  // the player's controls (API.md: Playing a country, Orders)
  if (on('[data-play-seat]')) { await order(`/api/play?tag=${encodeURIComponent(S?.country?.tag || '')}`); return refreshResearch(true).then(drawAll); }
  if (on('[data-observe]')) { armySel = null; await order('/api/observe'); return refreshResearch(true).then(drawAll); }
  if ((b = on('[data-tech]'))) { await order(`/api/research?tech=${encodeURIComponent(b.dataset.tech)}`); return refreshResearch(true).then(drawAll); }
  // events, decisions, diplomacy (API.md: Events and decisions, Diplomacy)
  if ((b = on('[data-event-key]'))) return order(`/api/event?key=${b.dataset.eventKey}&option=${b.dataset.option}`);
  if ((b = on('[data-decide]'))) return order(`/api/decision?name=${encodeURIComponent(b.dataset.decide)}`);
  if ((b = on('[data-answer]'))) return order(`/api/answer?key=${b.dataset.answer}&accept=${b.dataset.accept}`);
  if (on('[data-popup-hide]')) { popupHidden = true; return drawAll(); }
  if (on('[data-popup-show]')) { popupHidden = false; await refreshPlay(true); return drawAll(); }
  if (on('[data-justify]')) return order(`/api/justify?target=${encodeURIComponent(DIPSEL.target)}&cb=${encodeURIComponent(DIPSEL.cb || (DIPLO?.cbs || [])[0]?.key || '')}`);
  if ((b = on('[data-declare]'))) return order(`/api/war/declare?target=${encodeURIComponent(DIPSEL.target)}&cb=${encodeURIComponent(b.dataset.declare)}`);
  if ((b = on('[data-peace-goal]'))) { const s = DIPSEL.goals[b.dataset.war] ||= new Set(), g = +b.dataset.peaceGoal; s.has(g) ? s.delete(g) : s.add(g); return drawAll(); }
  if ((b = on('[data-offer-peace]'))) {
    const war = b.dataset.offerPeace, w = (DIPLO?.wars || []).find(x => x.name === war), to = DIPSEL.peaceTo[war] || (w?.talk || [])[0] || '';
    const goals = [...(DIPSEL.goals[war] || [])].join(',');
    DIPSEL.goals[war] = new Set();
    return order(`/api/peace/offer?war=${encodeURIComponent(war)}&to=${encodeURIComponent(to)}&goals=${goals}`);
  }
  if ((b = on('[data-army]'))) { const id = +b.dataset.army; armySel = armySel === id ? null : id; return drawAll(); }
  if ((b = on('[data-slider]'))) return moveSlider(b, e);
  if (on('[data-close]')) { open = null; return drawAll(); }
  if (on('[data-pause]')) { await tell(`/api/clock?paused=${!(S?.clock?.paused)}`); return refresh(); }
  if ((b = on('[data-speed]'))) { await tell(`/api/clock?speed=${b.dataset.speed}`); return refresh(); }
  if (on('[data-unselect]')) { selected = null; return refresh(); }
  if (on('[data-seat]')) { $('start').hidden = false; return; }
  if ((b = on('[data-mode]'))) { MAP.mode = b.dataset.mode; return refreshMap(true); }
  if ((b = on('[data-layer]'))) { const l = b.dataset.layer; MAP.shown.has(l) ? MAP.shown.delete(l) : MAP.shown.add(l); return refreshMap(true); }
  if (on('[data-fit]')) return fitMap(true);
});
// the Diplomacy window's choices: which country, which casus belli, whom to offer peace
document.addEventListener('change', e => {
  const t = e.target;
  if (t.matches('[data-dip-target]')) { DIPSEL.target = t.value; drawAll(); }
  else if (t.matches('[data-dip-cb]')) DIPSEL.cb = t.value;
  else if (t.matches('[data-peace-to]')) { DIPSEL.peaceTo[t.dataset.war] = t.value; drawAll(); }
});
document.addEventListener('keydown', e => {
  if (!$('start').hidden) return;
  if (e.key === 'Escape' && armySel != null) { armySel = null; return drawAll(); }
  if (e.key === 'Escape' && open) { open = null; return drawAll(); }
  if (e.key === ' ') { e.preventDefault(); tell(`/api/clock?paused=${!(S?.clock?.paused)}`).then(refresh); return; }
  // Map modes on the original's keys, for the modes this build can draw.
  const byKey = { w: 'political', s: 'population', d: 'culture', t: 'states' }[e.key];
  if (byKey && !open && MAP.modes.includes(byKey)) { MAP.mode = byKey; return refreshMap(true); }
  const t = TABS.find(x => x.key === e.key);
  if (t) { e.preventDefault(); open = open === t.id ? null : t.id; drawAll(); }
});

// ---- a budget slider of the played seat: an arrow moves it a step, a click on the track puts it there.
// The tariff's track runs from -100% to +100%; the others from 0 to 100%.
const SLIDER_STEP = 0.05;
function moveSlider(el, e) {
  const name = el.dataset.slider, tariff = name === 'tariff', at = el.dataset.at === '' ? 0.5 : +el.dataset.at;
  let share;
  const step = e.target.closest('[data-step]');
  if (step) share = at + SLIDER_STEP * +step.dataset.step * (tariff ? 0.5 : 1);
  else { const t = el.querySelector('.track').getBoundingClientRect(); share = (e.clientX - t.left) / Math.max(1, t.width); }
  share = Math.min(1, Math.max(0, share));
  const value = tariff ? share * 2 - 1 : share;
  return order(`/api/budget?slider=${encodeURIComponent(name)}&value=${value.toFixed(3)}`);
}

// ---- the map: drag to move, wheel to zoom, click to pick a province
const view = { x: 0, y: 0, k: 1 };
const img = $('mapimg'), mapEl = $('map');
const paper = $('mappaper');
// One paper tile covers PAPER_TILE map pixels (our choice), whatever the picture's width.
const PAPER_TILE = 256;
function place() {
  const t = `translate(${view.x}px, ${view.y}px) scale(${view.k})`;
  img.style.transform = t;
  if (!img.naturalWidth) return;
  paper.style.transform = t; paper.style.width = img.naturalWidth + 'px'; paper.style.height = img.naturalHeight + 'px';
  paper.style.backgroundSize = `${PAPER_TILE * img.naturalWidth / (MAP.full_width || 5616)}px auto`;
  paper.hidden = !(PAPER_OK && MAP.shown.has('terrain'));
  const W = MAP.full_width || 5616, H = MAP.full_height || 2160;
  names.style.transform = t; names.setAttribute('width', img.naturalWidth); names.setAttribute('height', img.naturalHeight);
  names.setAttribute('viewBox', `0 0 ${W} ${H}`);
  names.style.display = MAP.mode === 'political' && MAP.shown.has('owners') ? '' : 'none';
}
// The country names (GET /api/labels): one letter per text, placed and turned along the country.
const names = $('maplabels');
let labelsVersion = -1;
async function refreshLabels(version) {
  if (version === labelsVersion) return;
  try {
    const got = await ask('/api/labels');
    labelsVersion = version;
    names.innerHTML = got.map(l => [...l.text].map((ch, i) => {
      const [x, y, a] = l.glyphs[i] || [0, 0, 0];
      return ch.trim() ? `<text x="${x}" y="${y}" font-size="${l.size}" transform="rotate(${(a * 180 / Math.PI).toFixed(1)} ${x} ${y})">${esc(ch)}</text>` : '';
    }).join('')).join('');
  } catch (e) { /* the next map refresh tries again */ }
}
let PAPER_OK = false;
{ const probe = new Image(); probe.onload = () => { PAPER_OK = true; place(); }; probe.src = '/map/paper.bmp'; }
// Opens on Europe, where most of the game happens; "Fit map" shows the whole world.
const EUROPE = { x: 0.536, y: 0.235, share: 0.19 };   // centre and width of the opening view, as shares of the world picture
let placed = false;
function fitMap(world = false) {
  if (!img.naturalWidth) return;
  const w = img.naturalWidth, h = img.naturalHeight;
  if (world) { view.k = Math.min(innerWidth / w, innerHeight / h); view.x = (innerWidth - w * view.k) / 2; view.y = (innerHeight - h * view.k) / 2; }
  else { view.k = innerWidth / (w * EUROPE.share); view.x = innerWidth / 2 - w * view.k * EUROPE.x; view.y = innerHeight / 2 - h * view.k * EUROPE.y; }
  place();
}
img.addEventListener('load', () => { if (!placed) { placed = true; fitMap(); } else place(); });
let drag = null;
mapEl.addEventListener('pointerdown', e => { drag = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, moved: false }; mapEl.setPointerCapture(e.pointerId); });
mapEl.addEventListener('pointermove', e => { if (!drag) return; const dx = e.clientX - drag.x, dy = e.clientY - drag.y; if (Math.abs(dx) + Math.abs(dy) > 4) drag.moved = true; view.x = drag.vx + dx; view.y = drag.vy + dy; place(); });
mapEl.addEventListener('pointerup', async e => {
  const d = drag; drag = null; if (!d || d.moved || !img.naturalWidth) return;
  const x = (e.clientX - view.x) / (img.naturalWidth * view.k), y = (e.clientY - view.y) / (img.naturalHeight * view.k);
  if (x < 0 || x > 1 || y < 0 || y > 1) return;
  try {
    const p = await ask(`/api/pick?x=${x.toFixed(5)}&y=${y.toFixed(5)}`);
    // An army picked in the outliner goes where the player clicks; otherwise the click picks the province.
    if (armySel != null && p.province != null) { const id = armySel; armySel = null; return order(`/api/army/move?army=${id}&to=${p.province}`); }
    selected = p.province ?? null; refresh();
  } catch (err) { /* ignored: the next click tries again */ }
});
mapEl.addEventListener('wheel', e => {
  e.preventDefault(); const f = e.deltaY < 0 ? 1.2 : 1 / 1.2, k = Math.min(40, Math.max(0.05, view.k * f));
  view.x = e.clientX - (e.clientX - view.x) * (k / view.k); view.y = e.clientY - (e.clientY - view.y) * (k / view.k); view.k = k; place();
}, { passive: false });

drawMini(); drawAll(); start();
