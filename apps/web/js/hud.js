// THE MAIN SCREEN'S FIXED PARTS, top to bottom. Each draw function reads S (js/live.js) and paints
// one part. To change what a part shows, edit its function here.
let open = null;   // the window that is open, or null

// ---- top bar: eight windows, each with its summary underneath
const ALERTS = { factory_building: ['B', '#6fd24a', 'Factories are being built'], factory_closed: ['C', '#e04a3a', 'Closed factories'], unemployed: ['U', '#e0b03a', 'Unemployed workers'], can_reform: ['R', '#6fd24a', 'A reform can be passed'], can_decide: ['D', '#6fd24a', 'A decision can be taken'], election: ['E', '#e0b03a', 'An election is under way'], rebels: ['!', '#e04a3a', 'Rebels are in the country'], at_war: ['W', '#e04a3a', 'At war'], crisis: ['!', '#e0b03a', 'A crisis is under way'], losing_great_power: ['G', '#e04a3a', 'Great-power status is at risk'] };
const alerts = (c, ...names) => names.filter(n => (c.alerts || []).includes(n)).map(n => alertChip(...ALERTS[n], n)).join('');
const goods = list => (list || []).slice(0, 5).map(g => it(g, '', g)).join('');
const TABS = [
  { id: 'production', label: 'Production', key: 'F1', icon: 'industry', sum: c => goods(c.produces) + alerts(c, 'factory_building', 'factory_closed', 'unemployed') },
  { id: 'budget', label: 'Budget', key: 'F2', icon: 'money', sum: c => it('money', `<b>${num(c.money, 1)}</b>`, 'Treasury') + `<span class="it" title="Daily balance">${signed(c.balance)}</span>` },
  { id: 'technology', label: 'Technology', key: 'F3', icon: 'research', sum: c => `<span class="it">${esc(c.research?.name || 'No research')}</span>${has(c.research?.progress) ? `<span class="bar" style="max-width:5rem"><b style="width:${Math.round(c.research.progress * 100)}%"></b></span>` : ''}` + it('literacy', pct(c.literacy), 'Literacy') + it('research', num(c.research?.points_a_day, 1), 'Research points a day') },
  { id: 'politics', label: 'Politics', key: 'F4', icon: 'politics', sum: c => `<span class="it">${esc(c.ruling_party || '')}</span>` + it('infamy', num(c.infamy, 1), 'Infamy') + alerts(c, 'can_reform', 'can_decide', 'election', 'rebels') },
  { id: 'population', label: 'Population', key: 'F5', icon: 'pops', sum: c => `<span class="it" title="Population"><b>${num(c.population)}</b></span>` + it('focus', has(c.focus_used) ? `${c.focus_used}/${c.focus_max ?? '?'}` : DASH, 'National focuses') + it('militancy', num(c.militancy, 2), 'Militancy') + it('consciousness', num(c.consciousness, 2), 'Consciousness') },
  { id: 'trade', label: 'Trade', key: 'F6', icon: 'trade', sum: c => `<span class="it small">Exports</span>${goods(c.exports)}<span class="it small">Imports</span>${goods(c.imports)}` },
  { id: 'diplomacy', label: 'Diplomacy', key: 'F7', icon: 'diplomacy', sum: c => it('diplopts', num(c.diplomatic_points, 1), 'Diplomatic points') + alerts(c, 'at_war', 'crisis', 'losing_great_power') },
  { id: 'military', label: 'Military', key: 'F8', icon: 'military', sum: c => it('army', num(c.regiments), 'Regiments') + it('navy', num(c.ships), 'Ships') + it('manpower', num(c.manpower), 'Manpower') + it('leadership', num(c.leadership), 'Leadership') },
];
function drawTop() {
  const c = S?.country || {};
  paint($('top'), TABS.map(t => `<div class="tab"><button class="tabbtn" data-win="${t.id}" aria-pressed="${open === t.id}">${ic(t.icon)}${t.label}<kbd>${t.key}</kbd></button><div class="sum">${t.sum(c)}</div></div>`).join(''));
}

// ---- the country block and the clock
const SPEEDS = 5;
function drawIdent() {
  const c = S?.country || {}, k = S?.clock || {};
  paint($('ident'), `<div class="flag" title="${esc(c.tag || '')}"><span class="tagname">${esc(c.tag || '')}</span><span class="rank">${c.rank ?? ''}</span></div>
    <div><b>${esc(c.name || 'No country')}</b> <button class="mini" data-seat title="Look at another country">change</button>${!c.tag ? ''
        : c.played ? `<span class="played">you play it</span><button class="mini" data-observe title="Give every country to the computer and watch">Observe</button>`
        : `<button class="mini" data-play-seat title="Take control: the computer stops playing this country">Play this country</button>`}
      <div class="scores"><span title="Prestige">${ic('prestige')}${num(c.prestige)}</span><span title="Industry">${ic('industry')}${num(c.industry)}</span><span title="Military">${ic('military')}${num(c.military)}</span></div>
      <div id="clock"><button class="rnd" data-pause aria-label="${k.paused ? 'Unpause' : 'Pause'}">${k.paused ? '&#9654;' : '&#10074;&#10074;'}</button><span class="date">${esc(prettyDate(k.date))}</span>
        <button class="rnd" data-speed="${Math.max(1, (k.speed || 1) - 1)}" aria-label="Slower">&minus;</button><span class="speed" title="${has(k.days_per_second) ? k.days_per_second.toFixed(1) + ' days a second' : ''}">${Array.from({ length: SPEEDS }, (_, i) => `<i class="${!k.paused && i < (k.speed || 0) ? 'on' : ''}"></i>`).join('')}</span><button class="rnd" data-speed="${Math.min(SPEEDS, (k.speed || 1) + 1)}" aria-label="Faster">+</button></div>
      ${c.played && (c.waiting?.events || c.waiting?.questions) ? `<button class="mini" data-popup-show title="Events and questions waiting for your answer">${(c.waiting.events || 0) + (c.waiting.questions || 0)} waiting</button>` : ''}
      ${notice ? `<div class="small notice">${notice}</div>` : ''}
      ${online ? '' : '<div class="bad small">The game is not answering.</div>'}</div>`);
}
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
function prettyDate(d) { const m = /^(\d+)\.(\d+)\.(\d+)/.exec(d || ''); return m ? `${+m[3]} ${MONTHS[+m[2] - 1]} ${m[1]}` : ''; }

// ---- province window
function drawProv() {
  const p = S?.province, el = $('prov');
  el.hidden = !p; if (!p) return;
  const lvl = (n, max = 6) => `<span class="lvl">${Array.from({ length: max }, (_, i) => `<i class="${i < (n || 0) ? 'on' : ''}"></i>`).join('')}</span>`;
  paint(el, `<header class="burg"><div class="names"><small>${esc(p.state || '')}</small><b>${esc(p.name || '')}</b></div>
      <div class="stat" title="Owner">${esc(p.owner || '')}${p.controller && p.controller !== p.owner ? `<br><span class="bad">held by ${esc(p.controller)}</span>` : ''}</div>
      <div class="stat" title="Administrative efficiency">${ic('admin')}<br>${pct(p.admin_efficiency)}</div>
      <button class="close" data-unselect aria-label="Close province">&times;</button></header>
    <div class="terrain"><span>Life rating ${num(p.life_rating)}</span><span>${esc(p.terrain || '')}</span></div>
    <div class="grid">
      ${ic(p.goods || 'goods')}<span>${esc(p.goods || 'No goods')}${has(p.output) ? `, ${num(p.output, 1)} made` : ''}</span><span></span><span class="value">${money(p.income)}</span>
      ${ic('pops')}<span>Population</span><span></span><b>${has(p.population) ? Math.round(p.population).toLocaleString('en') : DASH}</b>
      ${ic('growth')}<span>Growth this month</span><span></span>${signed(p.growth, 0)}
    </div>
    <div class="pies"><div>${pie(p.workforce)}Workforce</div><div>${pie(p.ideology)}Ideology</div><div>${pie(p.culture)}Culture</div></div>
    <div class="grid" style="grid-template-columns:auto 1fr auto 1fr"><span>Supply limit</span><b>${num(p.supply_limit)}</b><span>Cores</span><span>${esc((p.cores || []).join(' '))}</span></div>
    <div class="build">${[['Fort', p.fort, 'fort'], ['Naval base', p.naval_base, 'naval_base'], ['Railroad', p.railroad, 'railroad']].map(([n, l, k]) => `<div class="row b"><span class="n">${ic(k)}${n}</span>${lvl(l)}</div>`).join('')}</div>`);
}

// ---- outliner
// While the seat is played an army can be picked here, then sent by a click on the map (js/main.js).
function drawOut() {
  const o = S?.outliner || {}, played = !!S?.country?.played;
  const group = (title, rows, line, hint = '') => `<h4><span>${title}</span><span>${(rows || []).length}</span></h4>${hint}<ul>${(rows || []).slice(0, 12).map(line).join('') || '<li><small>none</small></li>'}</ul>`;
  const army = a => `<li ${played ? `data-army="${a.id}" aria-pressed="${armySel === a.id}" title="Pick, then click a province to send it there"` : ''}><span>${esc(a.name)}</span><small>${num(a.men)}${a.to != null ? ' &rarr;' : ''}</small></li>`;
  const hint = !played ? '' : armySel != null ? '<div class="hint">Click a province to send the army; Esc to cancel.</div>' : '<div class="hint">Pick an army to move it.</div>';
  $('map').classList.toggle('aiming', played && armySel != null);
  paint($('out'), group('Wars', o.wars, w => `<li><span>${esc(w.name || 'War')}</span><small>${esc((w.against || []).join(' '))}</small></li>`)
    + group('Armies', o.armies, army, hint)
    + group('Navies', o.navies, n => `<li><span>${esc(n.name)}</span><small>${num(n.ships)} ships</small></li>`)
    + group('Factories being built', o.factories_building, f => `<li><span>${esc(String(f.kind || '').replace(/_/g, ' '))}</span><small>${has(f.days) ? num(f.days) + ' days' : ''}</small></li>`));
}

// ---- map modes
function drawMini() {
  paint($('mini'), `<div class="modes">${MAP.modes.map(m => `<button class="mode wide" aria-pressed="${m === MAP.mode}" data-mode="${esc(m)}">${esc(m)}</button>`).join('')}</div>
    <div class="modes" title="Map layers">${MAP.layers.map(l => `<button class="mode wide" aria-pressed="${MAP.shown.has(l)}" data-layer="${esc(l)}">${esc(l)}</button>`).join('')}</div>
    <div style="display:flex;gap:.3rem;justify-content:space-between">${btn('Whole world', 'sm', 'data-fit')}<span class="small" style="align-self:center">drag to move, wheel to zoom</span></div>`);
}

// ---- message log
function drawLog() {
  const rows = (S?.log || []).slice(0, 6);
  paint($('log'), rows.length ? rows.map(r => `<p><small>${esc(prettyDate(r.date))}</small> ${esc(r.text)}</p>`).join('') : '<p><small>Nothing to report yet. Unpause to let time pass.</small></p>');
}

function drawWindow() {
  const v = $('veil');
  if (!open) { v.className = ''; paint(v, ''); return; }
  v.className = 'on';
  paint(v, WIN[open] ? WIN[open](S || {}) : notBuilt(TABS.find(t => t.id === open)?.label || open));
}
function drawAll() { drawTop(); drawIdent(); drawProv(); drawOut(); drawLog(); drawWindow(); drawPopup(); }
