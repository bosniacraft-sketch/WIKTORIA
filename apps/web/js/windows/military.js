// MILITARY. One window = one file. It reads MILITARY (GET /api/military, asked by refreshMilitary
// while this window is open): the seat's leaders, armies, navies, the units being built, the unit
// types it could build, its mobilisation and its military numbers.
// Left: leaders (tabs all / generals / admirals) and the numbers. Middle: armies, then mobilisation.
// Right: navies. Build army / Build navy open the unit picker in place of the list.
// Live while the seat is played: a click on an army picks it for the move order (the same pick as
// the outliner's, js/main.js `data-army`) and closes the window, so the next click on the map sends
// it. Building, mobilising and leaders take no orders yet: drawn, disabled, titled "not yet".
let MILITARY = null;
const MILSEL = { leaders: 'all', picker: null };   // leaders tab; picker: null, 'land' or 'naval'

let militaryAsked = 0;
async function refreshMilitary(force = false) {
  if (open !== 'military') { MILITARY = null; return; }
  if (!force && Date.now() - militaryAsked < 1500) return;
  militaryAsked = Date.now();
  try { MILITARY = await ask('/api/military'); } catch (e) { /* the next round tries again */ }
}

// The window's own choices (tabs, picker); an army picked here also closes the window. This listener
// is added before js/main.js's, so it runs first; main.js then picks the army.
document.addEventListener('click', e => {
  const t = e.target, b = t.closest('[data-mil-tab], [data-mil-picker], [data-mil-close]');
  if (!b) return;
  if (b.dataset.milTab) { MILSEL.leaders = b.dataset.milTab; drawWindow(); }
  else if (b.dataset.milPicker !== undefined) { MILSEL.picker = MILSEL.picker === b.dataset.milPicker || !b.dataset.milPicker ? null : b.dataset.milPicker; drawWindow(); }
  else if (b.dataset.milClose !== undefined) open = null;
});

const NOT_YET = 'not yet: the game takes no such order from a player';
const milBar = (share, cls = '', what = '') => `<span class="bar mil-bar ${cls}" title="${what ? what + ' ' : ''}${pct(share)}"><b style="width:${has(share) ? Math.round(share * 100) : 0}%"></b></span>`;

WIN.military = S => {
  const m = MILITARY;
  if (!m) { refreshMilitary(true).then(drawWindow); return frameWin('Military', `<div class="todo">Asking the game&hellip;</div>`); }
  const played = !!m.played, canMove = played && m.orders?.move_army;
  const st = m.stats || {}, mob = m.mobilisation || {};
  const note = played ? '' : `<p class="small mil-note">The computer plays this country and leads its forces. Play it to move its armies yourself.</p>`;

  // ---- left: leaders and the numbers
  const leaders = (m.leaders || []).filter(l => MILSEL.leaders === 'all' || (MILSEL.leaders === 'admirals') === (l.kind === 'admiral'));
  const count = k => (m.leaders || []).filter(l => l.kind === k).length;
  const tabs = [['all', 'All', (m.leaders || []).length], ['generals', 'Generals', count('general')], ['admirals', 'Admirals', count('admiral')]]
    .map(([id, label, n]) => `<span class="${MILSEL.leaders === id ? 'on' : ''}" data-mil-tab="${id}" role="tab" aria-selected="${MILSEL.leaders === id}">${label} ${n}</span>`).join('');
  const trait = ([k, v]) => `<span class="mil-trait ${v < 0 ? 'bad' : 'good'}" title="${esc(k)}">${v < 0 ? '&minus;' : '+'}${num(Math.abs(v), Math.abs(v) < 1 ? 2 : 0)} ${esc(k.slice(0, 3))}</span>`;
  const leaderRow = l => `<div class="mil-leader">${ic(l.kind === 'admiral' ? 'navy' : 'army')}
      <div class="mil-lmain"><b>${esc(l.name)}</b><span class="small">${esc(l.personality?.name || 'no personality')} &middot; ${esc(l.background?.name || 'no background')}</span>
        <span class="mil-traits">${(l.traits || []).map(trait).join('') || '<span class="small">no traits</span>'}</span></div>
      <div class="mil-lside small"><span title="Prestige">${ic('prestige')}${pct(l.prestige)}</span><span class="${l.assigned ? '' : 'mil-idle'}">${l.assigned ? esc(l.assigned.name) : 'unassigned'}</span></div></div>`;
  const leaderList = part('military.leaders.list', `<div class="tabs" role="tablist">${tabs}</div><div class="mil-list mil-leaders">${leaders.map(leaderRow).join('') || '<p class="small mil-note">No leaders.</p>'}</div>
      <div class="mil-acts">${btn('Recruit general', 'sm', `disabled title="${NOT_YET}"`)}${btn('Recruit admiral', 'sm', `disabled title="${NOT_YET}"`)}</div>`);
  const stat = (name, label, v, title = '') => part(`military.stats.${name}`, `<div title="${esc(title)}"><span>${label}</span><span>${v}</span></div>`);
  const stats = part('military.stats', band('Military') + `<div class="kv mil-stats">
      ${stat('war_exhaustion', 'War exhaustion', num(st.war_exhaustion, 2))}
      ${stat('leadership', 'Leadership', num(st.leadership, 0))}
      ${stat('supply_consumption', 'Supply consumption', pct(st.supply_consumption))}
      ${stat('org_regain', 'Organisation regain', has(st.org_regain) ? signed(st.org_regain * 100, 0) + '%' : DASH, 'what technologies add to the daily regain')}
      ${stat('start_experience', 'Unit start experience', num(st.start_experience, 1))}
      ${stat('recruit_time', 'Recruit time', has(st.recruit_time) ? pct(st.recruit_time) : DASH)}
      ${stat('combat_width', 'Combat width', num(st.combat_width, 0))}
      ${stat('dig_in_cap', 'Dig-in cap', num(st.dig_in_cap, 0))}
      ${stat('tactics', 'Military tactics', pct(st.tactics))}
      ${stat('forces', 'Brigades / ships', `${num(st.regiments)} / ${num(st.ships)}`)}
    </div>`);

  // ---- armies and navies
  const DOING = { moving: 'Moving', battle: 'In battle', siege: 'Sieging', retreating: 'Retreating', embarked: 'On a fleet', dug_in: 'Dug in', standing: 'Standing', at_sea: 'At sea', in_port: 'In port' };
  const doing = u => `<span class="mil-doing mil-${esc(u.doing)}">${DOING[u.doing] || esc(u.doing)}${u.to ? ` &rarr; ${esc(u.to.name)}` : ''}${u.doing === 'dug_in' ? ` ${num(u.dig_in)}` : ''}</span>`;
  const unitRow = (u, army) => {
    const pick = army && canMove && u.doing !== 'embarked';
    const attrs = pick ? `data-army="${u.id}" data-mil-close aria-pressed="${armySel === u.id}" role="button" tabindex="0" title="Pick, then click a province on the map to send it there"` : '';
    return `<div class="mil-unit ${pick ? 'mil-pick' : ''}" ${attrs}>
      <div class="mil-uhead"><b>${esc(u.name)}</b><span class="small">${esc(u.province?.name || '')}</span></div>
      <div class="mil-ubody">${ic(army ? 'army' : 'navy')}<span class="mil-num">${army ? num(u.men) : num(u.ships) + ' ships'}</span>
        <span class="mil-bars">${milBar(u.strength, '', 'Strength')}${milBar(u.organisation, 'org', 'Organisation')}</span></div>
      <div class="mil-ufoot small">${doing(u)}<span>${u.leader ? esc(u.leader) : '<span class="mil-idle">no leader</span>'}${!army && u.carrying ? ` &middot; carries ${u.carrying}` : ''}</span></div></div>`;
  };
  const building = land => (m.building || []).filter(b => b.land === land);
  const buildRows = list => list.length ? list.map(b => `<div class="mil-build"><span>${esc(b.kind_name || b.kind)}</span><span class="small">${esc(b.province?.name || '')}</span>${milBar(b.progress)}<span class="small">${b.ends ? esc(prettyDate(b.ends)) : DASH}</span></div>`).join('') : '<p class="small mil-note">Nothing being built.</p>';
  const picker = land => {
    const rows = (m.units || []).filter(u => u.land === land).map(u => `<tr class="${u.active ? '' : 'mil-locked'}" title="${u.active ? '' : 'not available from the start'}">
        <td>${u.kind ? ic(u.kind) : ''}${esc(u.name)}</td><td class="n">${has(u.cost) ? money(u.cost, 0) : DASH}</td><td class="n">${num(u.days)} d</td>
        <td class="small">${(u.goods || []).map(([g, n]) => `${esc(String(g).replace(/_/g, ' '))} ${num(n)}`).join(', ')}</td>
        <td>${btn('Build', 'sm', `disabled title="${NOT_YET}"`)}</td></tr>`).join('');
    return `<div class="mil-picker"><div class="mil-phead"><b>${land ? 'Build a brigade' : 'Build a ship'}</b>${btn('Back', 'sm', 'data-mil-picker=""')}</div>
      <div class="mil-list"><table class="tbl"><tr><th>Type</th><th>Cost</th><th>Time</th><th>Goods</th><th></th></tr>${rows || '<tr><td colspan="5">No unit types.</td></tr>'}</table></div></div>`;
  };
  const column = (land) => {
    const key = land ? 'land' : 'naval', list = land ? m.armies || [] : m.navies || [];
    const head = part(`military.${land ? 'armies' : 'navies'}.header`, band(`${land ? 'Armies' : 'Navies'} <span class="mil-count">${list.length}</span>`));
    const build = part(`military.${land ? 'armies' : 'navies'}.build`, `<div class="mil-acts">${btn(land ? 'Build army' : 'Build navy', 'sm', `data-mil-picker="${key}" aria-pressed="${MILSEL.picker === key}" title="the unit types, their cost and time"`)}</div>`);
    const body = MILSEL.picker === key ? part(`military.${land ? 'armies' : 'navies'}.picker`, picker(land))
      : part(`military.${land ? 'armies' : 'navies'}.list`, `<div class="mil-list mil-units">${list.map(u => unitRow(u, land)).join('') || `<p class="small mil-note">No ${land ? 'armies' : 'navies'}.</p>`}</div>`);
    const queue = part(`military.${land ? 'armies' : 'navies'}.building`, `<div class="mil-sub">${land ? 'Brigades' : 'Ships'} being built</div>${buildRows(building(land))}`);
    return head + build + body + queue;
  };
  const hint = canMove ? `<p class="small mil-note">${armySel != null ? 'An army is picked: click a province on the map (Esc cancels).' : 'Click an army to pick it, then click a province on the map to send it.'}</p>` : '';
  const mobState = mob.mobilised ? (has(mob.progress) ? `Mobilising ${milBar(mob.progress)} ${pct(mob.progress)}` : '<b class="bad">Mobilised</b>') : 'Not mobilised';
  const mobil = part('military.mobilise', `<div class="row mil-mob"><span class="rowhead">Mobilisation</span><div class="rowbody">
      <span style="flex:1">${mobState}</span>
      ${it('army', `<b>${mob.mobilised ? DASH : num(mob.regiments)}</b>`, 'Brigades it would raise now')}<span class="small" title="Mobilisation size">size ${pct(mob.size, 1)}</span>
      <span class="small" title="Share of production lost while mobilised">production ${has(mob.production_loss) ? '&minus;' + pct(mob.production_loss, 1) : DASH}</span>
      ${btn(mob.mobilised ? 'Demobilise' : 'Mobilise', 'sm', `disabled title="${NOT_YET}"`)}</div></div>`);
  return frameWin('Military', `${note}<div class="mil-grid">
    <div class="col mil-col">${part('military.leaders.header', band('Leaders'))}${leaderList}${stats}</div>
    <div class="col mil-col">${column(true)}${hint}${mobil}</div>
    <div class="col mil-col">${column(false)}</div>
  </div>`);
};
