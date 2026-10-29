// THE OVERTURE: a small green terminal over the page while the engine answers its first questions,
// and a shorter one while the map is mounted. Two kinds of line, kept apart on purpose:
//   real    = made from the engine's answers (found the game, how many countries, the map's size)
//   flavour = theatre, states nothing, marked F below
// Any key or click skips it. It never claims a check it did not make.
const Boot = (() => {
  const el = () => document.getElementById('boot');
  const tube = () => el().querySelector('.tube');
  let skip = false, running = false;
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const dots = (label, result, w = 46) => label + ' ' + '.'.repeat(Math.max(3, w - label.length)) + ' ' + result;
  function put(text, cls) { const d = document.createElement('div'); if (cls) d.className = cls; d.textContent = text || ' '; tube().appendChild(d); return d; }
  async function type(text, cls, speed = 5) {
    const d = put('', cls);
    for (let i = 0; i < text.length && !skip; i++) { d.textContent += text[i]; if (i % 2 === 0) await sleep(speed); }
    d.textContent = text || ' ';
  }
  async function play(lines) {
    // lines: [text, cls, pauseAfter]
    running = true; skip = false;
    for (const [t, cls, pause] of lines) {
      if (skip) { put(t, cls); continue; }
      await type(t, cls, cls === 'big' ? 14 : 4);
      if (pause) await sleep(pause);
    }
    running = false;
  }
  function show(title) {
    const b = el(); b.hidden = false; b.classList.remove('fade');
    tube().innerHTML = ''; put(title, 'head');
    const s = document.createElement('div'); s.className = 'skip'; s.textContent = 'any key skips'; tube().appendChild(s);
  }
  async function hide() { const b = el(); b.classList.add('fade'); await sleep(950); b.hidden = true; }
  document.addEventListener('keydown', () => { if (running) skip = true; });
  document.addEventListener('mousedown', () => { if (running) skip = true; });

  // the first boot: shown at once, finished when the engine has answered (start() calls ready)
  let waiting = null, readyData = null;
  async function first() {
    show('WIKTORIA ENGINE  ::  TERMINAL 02');
    await play([
      [dots('WARMING THE TUBES', 'WARM'), 'dim', 120],                                   // F
      [dots('CALL Breathe()', 'UNAVAILABLE'), '', 160],                                   // F
      [dots('LOADING driver: clausewitz.sys', 'NOT OURS. LEFT AT THE DOOR.'), '', 160],   // F
    ]);
    waiting = put(dots('ASKING THE ENGINE', ''), '');
    let n = 0;
    while (!readyData && !skip) { waiting.textContent = dots('ASKING THE ENGINE', '.'.repeat(1 + n++ % 4)); await sleep(250); }
    const r = readyData || {};
    waiting.textContent = dots('ASKING THE ENGINE', r.install ? 'ANSWERING' : 'NO ANSWER YET');
    const land = (r.countries || []).filter(c => c.exists).length;
    await play([
      [dots('VICTORIA 2', r.install && r.install.ok ? 'FOUND' + (r.install.version ? ', VERSION ' + r.install.version : '') : 'NOT FOUND'), '', 120],   // real
      [dots('COUNTRIES', land ? land + ' OWN LAND' : '?'), '', 120],                                                                                        // real
      [dots('ASKING THE COMPUTER PLAYER FOR PEACE', 'REFUSED. OCCUPY EVERYTHING FIRST.'), '', 240],                                                        // F
      ['', '', 0],
      ['RUNS ON COMMUNITY POWER', 'big', 900],                                                                                                       // F
    ]);
    await hide();
  }
  function ready(data) { readyData = data; }

  // the second, shorter boot: while the map is mounted after the seat is chosen
  async function mounting(seatName) {
    show('WIKTORIA ENGINE  ::  MOUNTING THE WORLD');
    await play([
      [dots('SEAT', (seatName || '').toUpperCase()), '', 80],                                   // real
      [dots('MOUNTING THE MAP', ''), 'dim', 0],
    ]);
  }
  async function mounted(info) {
    const last = tube().lastChild;
    if (last && last.className !== 'skip') last.textContent = dots('MOUNTING THE MAP', info || 'MOUNTED');            // real
    await play([[dots('THE CLOCK', 'PAUSED. SPACE TO LET TIME PASS.'), '', 300], ['RUNS ON COMMUNITY POWER', 'big', 500]]);   // real, F
    await hide();
  }
  return { first, ready, mounting, mounted };
})();
Boot.first();
