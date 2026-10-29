// TRADE. One window = one file. It reads TRADE (GET /api/trade, asked here while this window is
// open): left every good, grouped, with its price and its trend against yesterday and a month ago;
// right the chosen good: its price history, the world's supply and demand, what the seat makes and
// buys of it, and tables under it. The engine has no trade orders for a player yet, so buying,
// selling, automating and the stockpile slider are drawn but disabled ("not yet").
let TRADE = null;
const TRADESEL = { good: null, tab: 'government', range: 365 };
const TRADE_DAYS = 1095;
let tradeAsked = 0, tradeBusy = false;

function askTrade(force = false) {
  if (tradeBusy || (!force && Date.now() - tradeAsked < 1500)) return;
  tradeBusy = true; tradeAsked = Date.now();
  const askedFor = TRADESEL.good, q = TRADESEL.good ? `good=${encodeURIComponent(TRADESEL.good)}&` : '';
  ask(`/api/trade?${q}days=${TRADE_DAYS}`).then(t => {
    TRADE = t;
    if (!TRADESEL.good) { const first = (t.goods || []).find(g => g.on_sale) || (t.goods || [])[0]; if (first) TRADESEL.good = first.key; }
    if (TRADESEL.good !== askedFor) tradeAsked = 0;   // chosen meanwhile: ask again at once
  }).catch(() => { /* the next round tries again */ }).finally(() => { tradeBusy = false; if (open === 'trade') { if (!tradeAsked) askTrade(true); drawAll(); } });
}

// A good's token: our icon where one is drawn, otherwise its two letters on the good's own colour
// (the colour comes from the install's goods file).
const goodToken = g => {
  const c = Array.isArray(g?.color) ? `rgb(${g.color.join(',')})` : '';
  return `<span class="gtok" style="${c ? `--gc:${c}` : ''}" title="${esc(g?.name || '')}">${ic(g?.key || '')}</span>`;
};
const units = v => has(v) ? num(v, 1) : DASH;
const priceTxt = v => has(v) ? (+v).toFixed(2) + ' &pound;' : DASH;

WIN.trade = S => {
  askTrade();
  const t = TRADE;
  if (!t) return frameWin('Trade', `<div class="trade"><div class="todo">Asking the game&hellip;</div></div>`);
  const sel = TRADESEL.good, played = !!t.played;

  // ---- left: the goods grid, by group in the file's order
  const groups = [];
  for (const g of t.goods || []) {
    const name = g.group_name || 'Other goods';
    let grp = groups.find(x => x.name === name);
    if (!grp) groups.push(grp = { name, key: g.group || 'other', list: [] });
    grp.list.push(g);
  }
  const cell = g => `<button class="gcell${g.key === sel ? ' on' : ''}${g.on_sale ? '' : ' off'}" data-trade-good="${esc(g.key)}" title="${esc(g.name)}${g.on_sale ? '' : ' (not on sale yet)'}">
      ${goodToken(g)}<span class="gname">${esc(g.name)}</span><span class="gprice">${has(g.price) ? (+g.price).toFixed(2) : DASH}</span>${trend(g.price, g.yesterday, 'since yesterday')}${trend(g.price, g.month, 'since a month ago')}</button>`;
  const grid = groups.map(grp => part(`trade.goods.${grp.key}`, band(esc(grp.name)) + `<div class="ggrid">${grp.list.map(cell).join('')}</div>`)).join('');
  const left = `<div class="tleft">${part('trade.goods.legend', `<div class="small glegend">Price in &pound;, then the change since yesterday and since a month ago.</div>`)}${grid}</div>`;

  // ---- right: the chosen good
  const d = t.good && t.good.key === sel ? t.good : null;
  const g = (t.goods || []).find(x => x.key === sel);
  let right;
  if (!g) right = `<div class="todo">Choose a good on the left.</div>`;
  else if (!d) right = `${part('trade.good.header', band(esc(g.name)))}<div class="todo">Asking the game&hellip;</div>`;
  else {
    const hist = d.history?.prices || [], shown = hist.slice(-TRADESEL.range);
    const fromDate = d.history?.from && shown.length ? dayAfter(d.history.from, hist.length - shown.length) : null;
    const ranges = [[30, 'Month'], [365, 'Year'], [TRADE_DAYS, 'All']].map(([n, l]) => `<button class="mini" data-trade-range="${n}" aria-pressed="${TRADESEL.range === n}">${l}</button>`).join('');
    const head = part('trade.good.header', `<div class="row"><div class="rowbody">${goodToken(g)}<b class="gtitle">${esc(d.name)}</b><span class="small">${esc(d.group_name || '')}</span>
      <span class="grow"></span><span class="small">base ${priceTxt(d.base)}</span><span class="value big">${priceTxt(d.price)}</span></div>
      <div class="rowbody small"><span>Since yesterday ${trend(d.price, d.yesterday)} ${change(d.price, d.yesterday)}</span><span>Since a month ago ${trend(d.price, d.month)} ${change(d.price, d.month)}</span></div></div>`);
    const chart = part('trade.good.history', `<div class="row"><span class="rowhead">Price history</span><span class="ranges">${ranges}</span><div class="chartbox">${lineChart(shown, { w: 600, h: 120, base: d.base, from: fromDate ? prettyDate(fromDate) : '', to: prettyDate(t.date), label: `${d.name} price`, empty: 'The history starts when the first day has run' })}</div></div>`);
    const w = d.world || {}, me = d.seat || {}, m = me.made || {}, b = me.bought || {};
    const kv = (label, v, title = '') => `<div title="${esc(title)}"><span>${label}</span><span>${v}</span></div>`;
    const world = part('trade.good.world', `${band('World market')}<div class="kv">
      ${kv('Supply', units(w.supply), 'Units offered on the markets yesterday')}${kv('Demand', units(w.demand), 'Units asked for and paid for yesterday')}
      ${kv('Demand served', pct(w.fill), 'Share of the paid demand the market could serve')}${kv('Supply sold', pct(w.sold), 'Share of the supply that found a buyer')}</div>`);
    const seat = part('trade.good.seat', `${band(`${esc(S.country?.name || t.tag)}: made and bought yesterday`)}<table class="tbl tsplit">
      <tr><th>Made</th><th>units</th><th>Bought</th><th>units</th></tr>
      <tr><td>Farms and mines</td><td class="n">${units(m.farms)}</td><td>Government</td><td class="n">${units(b.state)}</td></tr>
      <tr><td>Factories</td><td class="n">${units(m.factories)}</td><td>Factories</td><td class="n">${units(b.factories)}</td></tr>
      <tr><td>Artisans</td><td class="n">${units(m.artisans)}</td><td>Population</td><td class="n">${units(b.pops)}</td></tr>
      <tr class="tot"><td>Total</td><td class="n">${units(m.total)}</td><td>Total</td><td class="n">${units(b.total)}</td></tr></table>
      <div class="kv tkv">${kv('Own demand served', pct(me.fill))}${kv('Own goods sold', pct(me.sold))}${kv('Bought abroad', pct(me.imported), 'Share of what it bought that came from the world market')}${kv('Balance', has(m.total) && has(b.total) ? signed(m.total - b.total) : DASH, 'Made less bought, in units')}</div>`);
    const no = 'title="not yet: the game has no trade orders for a player"';
    const st = d.stockpile || {};
    const controls = part('trade.good.controls', `${band('Orders')}<div class="row"><div class="rowbody">
      ${btn('Buy', 'sm', `disabled ${no}`)}${btn('Sell', 'sm', `disabled ${no}`)}${btn('Automate', 'sm', `disabled ${no}`)}</div>
      <div class="rowbody"><span class="small">Stockpile</span><span class="tslider" ${no}>${slider(null)}</span><span class="value">${units(st.amount)}</span></div>
      ${played ? '' : `<p class="small tnote">The computer plays this country and does its buying and selling.</p>`}</div>`);
    right = `${head}${chart}<div class="tpair"><div>${world}${controls}</div><div>${seat}</div></div>${tables(d)}`;
  }
  return frameWin('Trade', `<div class="trade">${left}<div class="tright">${right}</div></div>`);
};

function change(now, before) { return has(now) && has(before) ? signed(now - before, 2) : DASH; }
// The date `n` days after `d` (the game's calendar has no leap days).
function dayAfter(d, n) {
  const LEN = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  let [y, m, day] = d.split('.').map(Number);
  for (let i = 0; i < n; i++) { if (day < LEN[m - 1]) day++; else if (m < 12) { m++; day = 1; } else { y++; m = 1; day = 1; } }
  return `${y}.${m}.${day}`;
}

// The tables under the chosen good, one at a time.
const TRADE_TABS = [['government', 'Government'], ['factories', 'Factories'], ['population', 'Population'], ['market', 'Market activity'], ['stockpile', 'Stockpile'], ['common', 'Common market']];
function tables(d) {
  const tab = TRADESEL.tab, row = cells => `<tr>${cells.join('')}</tr>`, td = (v, n = false) => `<td${n ? ' class="n"' : ''}>${v}</td>`;
  const tabs = `<div class="tabs ttabs">${TRADE_TABS.map(([k, l]) => `<button class="${k === tab ? 'on' : ''}" data-trade-tab="${k}" aria-pressed="${k === tab}">${l}</button>`).join('')}</div>`;
  const gov = d.government || {}, st = d.stockpile || {}, sl = st.sliders || {}, pop = d.population || {};
  let body = '';
  if (tab === 'government') body = `<table class="tbl"><tr><th>Government needs</th><th>units a day</th></tr>
    ${row([td('Army (at a full slider)'), td(units(gov.land), true)])}${row([td('Navy (at a full slider)'), td(units(gov.naval), true)])}
    ${row([td('Building work'), td(units(gov.construction), true)])}${row([td('Bought for the stockpile yesterday'), td(units(gov.bought), true)])}</table>`;
  else if (tab === 'factories') {
    const list = d.factories || [];
    body = list.length ? `<table class="tbl"><tr><th>Factory</th><th>State</th><th>Level</th><th>Makes</th><th>Uses</th><th></th></tr>
      ${list.map(f => row([td(esc(f.name)), td(esc(f.state || DASH)), td(num(f.level), true), td(f.makes ? units(f.made) : '', true), td(f.makes ? '' : units(f.used), true), td(f.building ? 'being built' : f.closed ? '<span class="bad">closed</span>' : '')])).join('')}</table>`
      : `<p class="small tnote">No factory of this country makes or uses this good.</p>`;
  } else if (tab === 'population') body = `<table class="tbl"><tr><th>Population</th><th>units yesterday</th></tr>
    ${row([td('Bought for needs and artisans\' inputs'), td(units(pop.bought), true)])}${row([td('Made by artisans'), td(units(pop.artisans_made), true)])}
    ${row([td('Needs by strata'), td(DASH, true)])}</table>`;
  else if (tab === 'market') {
    const side = (title, list) => `<table class="tbl"><tr><th>#</th><th>${title}</th><th>units</th><th>share</th></tr>${(list || []).map(r => row([td(r.rank, true), td(r.tag === TRADE.tag ? `<b>${esc(r.name)}</b>` : esc(r.name)), td(units(r.units), true), td(pct(r.share, 1), true)])).join('') || row([td(''), td(DASH), td(''), td('')])}</table>`;
    body = `<div class="tpair">${side('Largest makers', d.market?.makers)}${side('Largest buyers', d.market?.buyers)}</div>`;
  } else if (tab === 'stockpile') body = `<table class="tbl"><tr><th>National stockpile</th><th></th></tr>
    ${row([td('In store'), td(units(st.amount), true)])}${row([td('Army slider'), td(pct(sl.land), true)])}${row([td('Navy slider'), td(pct(sl.naval), true)])}${row([td('Building slider'), td(pct(sl.projects), true)])}</table>`;
  else if (tab === 'common') {
    const c = d.common;
    body = c ? `<table class="tbl"><tr><th>Sphere of ${esc((c.members || [])[0]?.name || c.leader)}</th><th>made</th><th>bought</th></tr>${(c.members || []).map((r, i) => row([td(`${i ? '' : '&#9733; '}${esc(r.name)}`), td(units(r.made), true), td(units(r.bought), true)])).join('')}</table>`
      : `<p class="small tnote">This country is in no sphere: it shares no common market.</p>`;
  }
  return part(`trade.good.tables.${tab}`, `<div class="ttables">${tabs}${body}</div>`);
}

// The window's own clicks: a good, a table tab, a history range.
document.addEventListener('click', e => {
  const b = e.target.closest('[data-trade-good],[data-trade-tab],[data-trade-range]');
  if (!b) return;
  if (b.dataset.tradeGood) { TRADESEL.good = b.dataset.tradeGood; drawAll(); askTrade(true); }
  else if (b.dataset.tradeTab) { TRADESEL.tab = b.dataset.tradeTab; drawAll(); }
  else if (b.dataset.tradeRange) { TRADESEL.range = +b.dataset.tradeRange; drawAll(); }
});
