// PRODUCTION. One window = one file. It reads PROD (GET /api/production, asked by this file while
// the window is open): the seat's states and their factory slots, money invested from abroad, what
// the country makes per good, the building work under way, and the factory types for the picker.
// The engine has no player order to build a factory yet (API.md: Production, can_build), so the
// picker's Build button is drawn but disabled ("not yet").
// PRODSEL keeps the open tab, whether the picker is open, and the type and state picked in it.
let PROD = null;
const PRODSEL = { tab: 'factories', picker: false, kind: '', state: null };
let prodAsked = 0, prodAsking = false;
async function refreshProduction(force = false) {
  if (open !== 'production') { PROD = null; return; }
  if (prodAsking || (!force && Date.now() - prodAsked < 1500)) return;
  prodAsked = Date.now(); prodAsking = true;
  try { PROD = await ask('/api/production'); drawAll(); } catch (e) { /* the next round tries again */ }
  prodAsking = false;
}
setInterval(refreshProduction, 1000);

document.addEventListener('click', e => {
  const b = e.target.closest('[data-prod-tab],[data-prod-picker],[data-prod-kind],[data-prod-close]');
  if (!b) return;
  if (b.dataset.prodTab) { PRODSEL.tab = b.dataset.prodTab; PRODSEL.picker = false; }
  else if (b.dataset.prodPicker !== undefined) { PRODSEL.picker = true; PRODSEL.state = b.dataset.prodPicker === '' ? null : +b.dataset.prodPicker; }
  else if (b.dataset.prodKind) PRODSEL.kind = b.dataset.prodKind;
  else if (b.dataset.prodClose !== undefined) PRODSEL.picker = false;
  drawAll();
});
document.addEventListener('keydown', e => {
  if ((e.key === 'Enter' || e.key === ' ') && e.target.matches?.('[data-prod-tab]')) { e.preventDefault(); e.stopPropagation(); e.target.click(); }
}, true);

WIN.production = S => {
  const p = PROD;
  if (!p) { refreshProduction(true); return frameWin('Production', `<div class="todo">Asking the game&hellip;</div>`); }
  if (p.exists === false) return frameWin('Production', `<div class="todo">This country has no land.</div>`);
  const played = !!(S.country?.played ?? p.played);
  const tabs = [['factories', 'Factories'], ['investment', 'Foreign investment'], ['goods', 'Production'], ['projects', 'Projects']];
  const body = { factories: prodFactories, investment: prodInvestment, goods: prodGoods, projects: prodProjects }[PRODSEL.tab] || prodFactories;
  return frameWin('Production', `${part('production.tabs', tabStrip(tabs, PRODSEL.tab, 'data-prod-tab'))}
    <div class="prod">${body(p, played)}${PRODSEL.picker ? prodPicker(p, played) : ''}</div>`);
};

// ---- Factories: one row per state, its factory slots filled or empty.
function prodFactories(p, played) {
  const states = p.states || [], all = states.flatMap(s => s.factories || []), slots = p.max_per_state || 8;
  const workers = all.reduce((a, f) => a + (f.workers || 0), 0), profit = all.reduce((a, f) => a + (f.status === 'running' || f.status === 'building' ? f.profit || 0 : 0), 0);
  const who = p.policy ? [p.policy.build && 'the state builds', p.policy.pops_build && 'capitalists build', p.policy.subsidise && 'the state may subsidise'].filter(Boolean).join(', ') || 'nobody may build' : DASH;
  const card = f => {
    const st = { closed: '<span class="bad">Closed</span>', opening: `<span class="soon">Opens in ${num(f.days_left)} d</span>`, building: `<span class="soon">+1 level in ${num(f.days_left)} d</span>` }[f.status] || '';
    const tip = `${f.name}, level ${num(f.level)}\nWorkers ${num(f.workers)} of ${num(f.places)}\nSales ${num(f.income, 2)}, inputs ${num(f.inputs, 2)}, pay ${num(f.pay, 2)}\nMakes ${num(f.output, 2)} ${f.good_name || ''} a day${f.subsidised ? '\nSubsidised' : ''}${f.loss_days ? `\n${f.loss_days} days at a loss` : ''}\nThrottle: not in the engine`;
    return part(`production.factories.factory_${f.index}`, `<div class="slot ${f.status}" title="${esc(tip)}">
      <div class="top">${goodTile(f.good, f.good_name, (p.goods || []).find(g => g.key === f.good)?.colour)}<span class="lvl" title="Level">${num(f.level)}</span>${f.subsidised ? '<span class="sbd" title="Subsidised">S</span>' : ''}</div>
      <div class="nm">${esc(f.name)}</div>
      ${meter(f.workers, f.places, `Workers ${num(f.workers)} of ${num(f.places)}`)}
      <div class="ln"><span title="Workers">${num(f.workers)}</span>${f.status === 'closed' || f.status === 'opening' ? '' : signed(f.profit, 1)}</div>
      ${st ? `<div class="ln">${st}</div>` : `<div class="ln small" title="Throttle: not in the engine">thr. ${DASH}</div>`}</div>`);
  };
  const empty = s => `<div class="slot empty" data-prod-picker="${s.id ?? ''}" title="Build a factory here"><span>+</span></div>`;
  const rows = states.map(s => {
    const fs = s.factories || [], free = Math.max(0, slots - fs.length);
    return part(`production.factories.state_${s.id ?? 'none'}`, `<div class="row pstate">
      <div class="shead"><b>${esc(s.name)}</b>${s.colony ? ' <span class="small soon">colony</span>' : ''}
        <span class="small">${s.provinces} province${s.provinces === 1 ? '' : 's'} &middot; workforce ${num(s.workforce)}</span>
        <span class="small" title="What its capitalists have put aside to build">fund ${money(s.investment)}</span></div>
      <div class="slots" style="--n:${slots}">${fs.map(card).join('')}${s.colony ? (fs.length ? '' : '<span class="small nofac">A colony: no factories are built here.</span>') : Array.from({ length: free }, () => empty(s)).join('')}</div></div>`);
  }).join('');
  return `${part('production.factories.summary', `<div class="row"><div class="rowbody">
      ${it('industry', `<b>${all.length}</b>`, 'Factories')}<span class="small">in ${states.filter(s => (s.factories || []).length).length} of ${states.length} states</span>
      ${it('pops', num(workers), 'Factory workers')}<span title="Yesterday's profit of the running factories" style="white-space:nowrap">Profit ${signed(profit, 1)}</span>
      <span class="small" style="flex:1">Policy: ${who}</span>
      ${btn('Build factory', 'sm', 'data-prod-picker=""')}</div></div>`)}
    <div class="pstates">${rows || '<p class="small">No states.</p>'}</div>`;
}

// ---- Foreign investment: money other countries have put here, and ours abroad.
function prodInvestment(p) {
  const fi = p.foreign_investment;
  if (!fi) return `<div class="todo">The engine does not keep foreign investment. ${DASH}</div>`;
  const table = (rows, empty) => {
    const total = (rows || []).reduce((a, r) => a + (r.amount || 0), 0);
    return rows && rows.length ? `<table class="tbl"><tr><th>Country</th><th>Invested</th><th>Share</th></tr>${rows.slice().sort((a, b) => b.amount - a.amount).map(r => `<tr><td>${esc(r.name)}</td><td class="n">${money(r.amount)}</td><td class="n">${pct(total > 0 ? r.amount / total : null)}</td></tr>`).join('')}<tr><td><b>Total</b></td><td class="n"><b>${money(total)}</b></td><td></td></tr></table>` : `<p class="small" style="margin:.3rem .4rem">${empty}</p>`;
  };
  return `<div class="cols">
    <div class="col">${part('production.investment.here', band('Invested in us') + table(fi.here, 'No country has money invested here.'))}</div>
    <div class="col">${part('production.investment.abroad', band('Our investment abroad') + table(fi.abroad, 'We have no money invested abroad.'))}</div></div>
    <p class="small" style="margin:.4rem">As the save has it: the engine does not move foreign investment yet, and nothing can be invested from here.</p>`;
}

// ---- Production: a grid of goods per group, what the country makes of each a day.
function prodGoods(p) {
  const goods = p.goods || [];
  const tile = g => {
    const tip = `${g.name}: ${num(g.total, 2)} a day\nFarms and mines ${num(g.rgo, 2)}\nFactories ${num(g.factories, 2)}\nArtisans ${num(g.artisans, 2)}\nPrice ${num(g.price, 2)}`;
    return `<div class="pgood ${g.total > 0 ? '' : 'none'}" title="${esc(tip)}">${goodTile(g.key, g.name, g.colour)}<span class="nm">${esc(g.name)}</span><span class="v">${g.total > 0 ? num(g.total, 1) : DASH}</span></div>`;
  };
  const value = goods.reduce((a, g) => a + (g.total || 0) * (g.price || 0), 0);
  return `${part('production.goods.summary', `<div class="row"><div class="rowbody"><span style="flex:1">What the country makes a day, in units: farms and mines, factories and artisans.</span><span class="small">worth</span><span class="value">${money(value)}</span></div></div>`)}
    <div class="ggroups">${(p.groups || []).map(gr => part(`production.goods.${gr.key}`, `<div class="ggroup">${band(esc(gr.name))}<div class="ggrid">${goods.filter(g => g.group === gr.key).map(tile).join('')}</div></div>`)).join('')}</div>`;
}

// ---- Projects: what is being built, by whom, how far along, and the goods it still needs.
function prodProjects(p) {
  const rows = (p.projects || []).map(x => {
    const needs = (x.needs || []).map(n => `<span title="${esc(n.name)}">${goodTile(n.key, n.name, (p.goods || []).find(g => g.key === n.key)?.colour)}${num(n.amount, 1)}</span>`).join('');
    const what = { new: 'New', expand: 'Enlarge', reopen: 'Reopen' }[x.what] || x.what;
    const fund = has(x.price) ? `${money(x.paid)} of ${money(x.price)}` : DASH;
    return `<tr><td>${esc(x.state)}</td><td><b>${esc(x.name)}</b> <span class="small">${what}</span></td><td>${x.by === 'state' ? 'State' : 'Capitalists'}</td>
      <td><span class="pbar">${meter(x.progress, 1)}<span>${pct(x.progress)}</span></span></td><td class="n">${has(x.days_left) ? num(x.days_left) + ' d' : DASH}</td><td class="n">${fund}</td>
      <td><span class="needs">${needs || (x.by === 'state' ? '<span class="small">nothing</span>' : DASH)}</span></td></tr>`;
  }).join('');
  return part('production.projects', `${band('Projects')}${rows ? `<table class="tbl"><tr><th>State</th><th>Project</th><th>By</th><th>Progress</th><th>Left</th><th>Fund</th><th>Goods still needed</th></tr>${rows}</table>` : '<p class="small" style="margin:.3rem .4rem">Nothing is being built or planned.</p>'}
    <p class="small" style="margin:.4rem">A site the state pays for opens when its time is up and its goods have arrived; a capitalists' plan starts once its fund covers the price.</p>`);
}

// ---- Build factory: the list of types with cost and time, and on the right the picked type.
function prodPicker(p, played) {
  const kinds = (p.kinds || []).slice().sort((a, b) => (b.available - a.available) || a.name.localeCompare(b.name));
  const k = kinds.find(x => x.key === PRODSEL.kind) || kinds.find(x => x.available) || kinds[0];
  const state = (p.states || []).find(s => s.id === PRODSEL.state);
  const list = kinds.map(x => `<tr data-prod-kind="${esc(x.key)}" class="${x === k ? 'on' : ''} ${x.available ? '' : 'off'}" title="${x.available ? '' : 'Not yet invented'}">
      <td>${goodTile(x.good, x.good_name, (p.goods || []).find(g => g.key === x.good)?.colour)}</td><td>${esc(x.name)}</td><td class="n">${money(x.cost)}</td><td class="n">${num(x.days)} d</td></tr>`).join('');
  const goodsList = l => (l || []).map(g => `<div class="gl">${goodTile(g.key, g.name, (p.goods || []).find(x => x.key === g.key)?.colour)}<span>${esc(g.name)}</span><b>${num(g.amount, 2)}</b></div>`).join('') || `<div class="small">${DASH}</div>`;
  const detail = k ? `<div class="pk-head">${goodTile(k.good, k.good_name, (p.goods || []).find(g => g.key === k.good)?.colour)}<b>${esc(k.name)}</b><span class="small">makes ${esc(k.good_name || DASH)}</span></div>
      ${band('Inputs a day per level')}<div class="glist">${goodsList(k.inputs)}</div>
      ${band('Workers')}<div class="glist">${(k.jobs || []).map(j => `<div class="gl">${ic('pops')}<span>${esc(j.name)}</span><b>${num((k.workforce || 0) * j.share)}</b></div>`).join('')}
        ${k.owner ? `<div class="gl">${ic('pops')}<span>${esc(k.owner.name)} (owners)</span><b></b></div>` : ''}</div>
      ${band('To build')}<div class="glist">${goodsList(k.goods)}</div>
      <div class="kv" style="margin-top:.4rem"><div><span>Cost today</span><span>${money(k.cost)}</span></div><div><span>Time</span><span>${num(k.days)} days</span></div></div>` : '<div class="todo">No factory types.</div>';
  const why = !p.can_build ? 'not yet, the engine has no order to build a factory' : !played ? 'Play this country to build' : !k?.available ? 'Not yet invented' : '';
  return part('production.build', `<div class="picker paper frame">
    <div class="pk-title">${band(`Build factory${state ? ' in ' + esc(state.name) : ''}`)}<button class="close" data-prod-close aria-label="Close the picker">&times;</button></div>
    <div class="pk-cols"><div class="pk-list"><table class="tbl"><tr><th></th><th>Type</th><th>Cost</th><th>Time</th></tr>${list}</table></div>
      <div class="pk-detail">${detail}</div></div>
    <div class="foot"><span class="small" style="flex:1">Greyed types are not invented yet.${why ? ' Build: ' + esc(why) + '.' : ''}</span>${btn('Cancel', 'sm', 'data-prod-close')}${btn('Build', 'sm', `disabled title="${esc(why)}"`)}</div></div>`);
}
