// DIPLOMACY. One window = one file. It reads DIPLO (GET /api/diplomacy, asked by js/live.js while
// this window is open or a question waits). Top: tabs for the great powers, the wars (all of them,
// the seat's own with its goals and peace offers), war justification and the crisis. Under them the
// country table (region filters, sortable) and, right, the selected country with its actions.
// While the seat is played it can justify, declare war, offer peace and answer (js/main.js sends
// those orders); the other actions have no order in the engine yet and are drawn disabled.
// DIPSEL keeps what the player picked between two refreshes.
const DIPSEL = { target: '', cb: '', peaceTo: {}, goals: {}, tab: 'powers', filter: 'all', sort: 'rank', dir: 1 };   // goals: war name -> Set of goal indices
const DIP_LEVELS = ['Hostile', 'Opposed', 'Neutral', 'Cordial', 'Friendly', 'In sphere'];
const DIP_STATUS = { great: 'Great power', secondary: 'Secondary power', civilised: 'Civilised nation', uncivilised: 'Uncivilised nation' };

// One question (peace offer or call to arms) with its two answers; also used by the popup (js/hud.js).
function questionHtml(q, played) {
  const goals = (q.goals || []).map(g => `${esc(g.name)} (${esc(g.actor)} on ${esc(g.receiver)})`).join(', ');
  const what = q.kind === 'call' ? `<b>${esc(q.from_name)}</b> calls you to arms in <i>${esc(q.war)}</i>.`
    : `<b>${esc(q.from_name)}</b> offers peace in <i>${esc(q.war)}</i>: ${goals ? 'it enforces ' + goals : 'a white peace'}.`;
  const acts = played ? `${btn(q.kind === 'call' ? 'Join the war' : 'Accept', 'sm', `data-answer="${q.key}" data-accept="true"`)}${btn(q.kind === 'call' ? 'Refuse (the alliance ends)' : 'Refuse', 'sm', `data-answer="${q.key}" data-accept="false"`)}` : '';
  return `<div class="row q"><div class="rowbody"><span style="flex:1">${what} <span class="small">Answer by ${esc(prettyDate(q.expires))}, or the computer answers for you.</span></span>${acts}</div></div>`;
}

// Every country the window knows, by tag: the overview row merged with play.rs's relation record.
function dipNations(d) {
  const rel = Object.fromEntries((d.countries || []).map(c => [c.tag, c]));
  const out = {};
  for (const n of d.nations || []) out[n.tag] = { ...rel[n.tag], ...n, relation: rel[n.tag]?.relation ?? null, held: rel[n.tag]?.held || [] };
  for (const c of d.countries || []) out[c.tag] ||= { ...c, influence: [], levels: [] };
  return out;
}
const dipName = (all, d, tag) => all[tag]?.name || (d.powers || []).find(p => p.tag === tag)?.name || (tag === d.tag ? S?.country?.name : '') || tag;
const dipFlag = (all, d, tag, o = {}) => flagTile(tag, all[tag]?.colour || (d.powers || []).find(p => p.tag === tag)?.colour, { title: dipName(all, d, tag), ...o });
const dipLevel = v => has(v) ? `<span class="lv l${v}">${DIP_LEVELS[v]}</span>` : DASH;

// ---- the tabs
function dipPowers(d, all) {
  const cards = (d.powers || []).map(p => {
    const seat = p.tag === d.tag;
    const toUs = seat ? `<div class="small">Your country</div>` : `<div class="small">Of you: ${dipLevel(p.opinion)}, influence ${num(p.influence)}</div>`;
    return part('diplomacy.powers.card', `<button class="gpcard ${seat ? 'seat' : ''} ${p.tag === DIPSEL.target ? 'on' : ''}" data-dip-pick="${esc(p.tag)}">
      <div class="gphead">${flagTile(p.tag, p.colour, { rank: p.rank, size: 'lg', title: p.name })}<b>${esc(p.name)}</b></div>
      <div class="gpsc"><span>${ic('prestige')}${num(p.prestige)}</span><span>${ic('industry')}${num(p.industry)}</span><span>${ic('military')}${num(p.military)}</span></div>
      <div class="gpline"><span>Score <b>${num(p.total)}</b></span><span>Sphere ${num(p.sphere)}</span></div>
      <div class="gpline"><span>Relation ${seat ? DASH : signed(p.relation, 0)}</span></div>${toUs}</button>`);
  }).join('');
  return part('diplomacy.powers', `<div class="gpgrid">${cards || '<p class="small">No great powers.</p>'}</div>`);
}

function dipWarScore(score) {
  if (!has(score)) return `<span class="wsbar" title="War score not known"><i></i><em>${DASH}</em></span>`;
  const v = Math.max(-100, Math.min(100, score)), w = Math.abs(v) / 2;
  return `<span class="wsbar" title="War score ${num(score, 1)}% (attackers' view)"><b class="${v < 0 ? 'neg' : 'pos'}" style="${v < 0 ? `right:50%;width:${w}%` : `left:50%;width:${w}%`}"></b><i></i><em>${v < 0 ? '&minus;' : '+'}${num(Math.abs(v), 0)}%</em></span>`;
}

function dipWars(d, all, played) {
  const side = list => list.map((t, i) => `<span class="wside ${i ? '' : 'lead'}">${dipFlag(all, d, t, { size: 'sm' })}${esc(dipName(all, d, t))}</span>`).join('');
  const ours = Object.fromEntries((d.wars || []).map(w => [w.name, w]));
  const detail = w => {
    const sel = DIPSEL.goals[w.name] || new Set(), to = DIPSEL.peaceTo[w.name] || (w.talk || [])[0] || '';
    const goal = g => `<tr><td>${played && (w.talk || []).length ? `<button class="mini" data-peace-goal="${g.index}" data-war="${esc(w.name)}" aria-pressed="${sel.has(g.index)}">${sel.has(g.index) ? '&#10003;' : '&nbsp;'}</button>` : ''}</td>
      <td>${esc(g.name)}</td><td>${esc(g.actor)} &rarr; ${esc(g.receiver)}</td><td>${esc(g.province?.name || '')}</td><td>${g.ours ? 'ours' : 'theirs'}</td></tr>`;
    const offer = played && (w.talk || []).length ? `<div class="rowbody"><span>Offer peace to</span><select data-peace-to data-war="${esc(w.name)}">${w.talk.map(t => `<option value="${esc(t)}" ${t === to ? 'selected' : ''}>${esc(dipName(all, d, t))}</option>`).join('')}</select>
        <span class="small" style="flex:1">${sel.size ? 'enforcing the ticked goals (all ours, or all theirs)' : 'a white peace (tick goals to demand or give them)'}</span>${btn('Offer peace', 'sm', `data-offer-peace="${esc(w.name)}"`)}</div>` : '';
    return `${(w.goals || []).length ? `<table class="tbl"><tr><th></th><th>Goal</th><th>Who</th><th>State</th><th></th></tr>${w.goals.map(goal).join('')}</table>` : '<div class="rowbody small">No war goals left.</div>'}${offer}`;
  };
  const list = d.all_wars || (d.wars || []).map(w => ({ ...w, side: w.attacker ? 'attacker' : 'defender', score: has(w.score) ? (w.attacker ? w.score : -w.score) : null }));
  const rows = list.map(w => part('diplomacy.wars.war', `<div class="row war ${w.side ? 'ours' : ''}"><div class="wtop"><b>${esc(w.name)}</b>${w.side ? `<span class="small">you are ${w.side === 'attacker' ? 'an attacker' : 'a defender'}</span>` : ''}</div>
    <div class="wline"><div class="wsides">${side(w.attackers || [])}</div>${dipWarScore(w.score)}<div class="wsides def">${side(w.defenders || [])}</div></div>
    ${ours[w.name] ? detail(ours[w.name]) : ''}</div>`)).join('');
  return part('diplomacy.wars', rows || '<p class="small dipempty">No wars are being fought.</p>');
}

function dipJustify(d, all, played, target) {
  const j = d.justifying;
  const head = `<div class="rowbody"><span>Your infamy <b>${num(d.infamy, 1)}</b> of ${num(d.infamy_limit, 0)}</span><span class="bar inf" style="max-width:12rem"><b style="width:${has(d.infamy) && d.infamy_limit ? Math.min(100, d.infamy / d.infamy_limit * 100) : 0}%"></b></span></div>`;
  const justifying = j ? `<div class="rowbody"><span style="flex:1">Justifying <b>${esc(j.name)}</b> against ${esc(dipName(all, d, j.target))}${j.discovered ? ' <span class="bad">(discovered)</span>' : ''}</span><span class="bar" style="max-width:12rem"><b style="width:${Math.round((j.progress || 0) * 100)}%"></b></span><span>${pct(j.progress)}</span></div>`
    : '<div class="rowbody small">No casus belli is being justified.</div>';
  let country = `<div class="rowbody small">Pick a country in the table below.</div>`;
  if (target) {
    const held = (target.held || []).map(h => `<div class="rowbody"><span style="flex:1">Casus belli <b>${esc(h.name)}</b> on ${esc(target.name)}, until ${esc(prettyDate(h.until))}</span>${played && !target.at_war && !target.allied ? btn('Declare war', 'sm', `data-declare="${esc(h.cb)}"`) : ''}</div>`).join('')
      || `<div class="rowbody small">No casus belli against ${esc(target.name)}.</div>`;
    const cbs = d.cbs || [];
    const canJustify = played && !j && !target.at_war && !target.truce && cbs.length;
    const why = !played ? 'Play this country to justify a war.' : j ? 'One justification at a time.' : target.at_war ? 'You are at war with them.' : target.truce ? 'A truce holds.' : !cbs.length ? 'No casus belli can be justified.' : '';
    const justify = canJustify ? `<div class="rowbody"><span>Justify on ${esc(target.name)}</span><select data-dip-cb>${cbs.map(c => `<option value="${esc(c.key)}" ${c.key === DIPSEL.cb ? 'selected' : ''}>${esc(c.name)} (infamy ${num(c.infamy, 1)} if discovered)</option>`).join('')}</select>${btn('Justify', 'sm', 'data-justify')}</div>` : `<div class="rowbody small">${why}</div>`;
    country = held + justify;
  }
  return part('diplomacy.justify', `<div class="row">${head}${justifying}${country}</div>`);
}

function dipCrisis(d) {
  const c = d.crisis;
  return part('diplomacy.crisis', `<div class="row"><div class="rowbody"><b style="flex:1">${c ? esc(c.name || 'Crisis') : 'No crisis'}</b></div>
    <div class="kv"><div><span>Temperature</span><span>${c ? pct(c.temperature) : DASH}</span></div><div><span>Attacker</span><span>${c ? esc(c.attacker || '') : DASH}</span></div>
    <div><span>Defender</span><span>${c ? esc(c.defender || '') : DASH}</span></div><div><span>Backers</span><span>${c ? num((c.backers || []).length) : DASH}</span></div></div></div>`);
}

// ---- the country table
function dipFiltered(d, all) {
  const f = DIPSEL.filter, sphere = d.seat_great_power ? d.tag : d.sphere_of_seat;
  const keep = n => f === 'all' ? true : f.startsWith('cont:') ? n.continent === f.slice(5) : f === 'neighbours' ? n.neighbour
    : f === 'sphere' ? !!sphere && (n.sphere_of === sphere || n.tag === sphere) : f === 'enemies' ? n.at_war : f === 'allies' ? n.allied : true;
  const rows = Object.values(all).filter(keep);
  const k = DIPSEL.sort, dir = DIPSEL.dir;
  const val = n => k === 'name' ? n.name : k === 'rank' ? n.rank : k === 'relation' ? n.relation : k === 'opinion' ? n.opinion : k.startsWith('gp') ? n.influence?.[+k.slice(2)] : null;
  rows.sort((a, b) => {
    const x = val(a), y = val(b);
    if (x == null && y == null) return a.name.localeCompare(b.name);
    if (x == null) return 1; if (y == null) return -1;
    return (typeof x === 'string' ? x.localeCompare(y) : x - y) * dir || a.name.localeCompare(b.name);
  });
  return rows;
}

function dipTable(d, all) {
  const f = (key, label) => `<button class="dfilt ${DIPSEL.filter === key ? 'on' : ''}" data-dip-filter="${esc(key)}">${label}</button>`;
  const filters = part('diplomacy.table.filters', `<div class="dfilters">${f('all', 'All')}${(d.continents || []).map(c => f('cont:' + c, esc(c))).join('')}${f('neighbours', 'Neighbours')}${f('sphere', 'Sphere')}${f('enemies', 'Enemies')}${f('allies', 'Allies')}</div>`);
  const powers = d.powers || [];
  const th = (key, label, title = '') => `<th data-dip-sort="${key}" title="${esc(title)}" class="${DIPSEL.sort === key ? 'on' : ''}">${label}${DIPSEL.sort === key ? (DIPSEL.dir > 0 ? ' &#9650;' : ' &#9660;') : ''}</th>`;
  const head = `<tr><th></th>${th('name', 'Country')}${th('rank', 'Rank')}${th('relation', 'Relation', 'Relation with you')}${th('opinion', 'Opinion', 'Your opinion of them (great powers only)')}${powers.map((p, i) => th('gp' + i, flagTile(p.tag, p.colour, { size: 'sm', title: '' }), `${p.name}: influence (and opinion)`)).join('')}</tr>`;
  const rows = dipFiltered(d, all).map(n => {
    const gp = powers.map((p, i) => { const lv = n.levels?.[i]; return `<td class="n gpi ${has(lv) ? 'l' + lv : ''}" title="${esc(p.name)}: ${has(lv) ? DIP_LEVELS[lv] : 'opinion not known'}">${p.tag === n.tag ? '' : num(n.influence?.[i])}</td>`; }).join('');
    const marks = (n.at_war ? '<span class="bad" title="At war">&#9876;</span>' : '') + (n.allied ? '<span class="good" title="Ally">&#9775;</span>' : '');
    return `<tr data-dip-pick="${esc(n.tag)}" class="${n.tag === DIPSEL.target ? 'on' : ''}"><td>${flagTile(n.tag, n.colour, { size: 'sm', title: n.name })}</td><td class="nm">${esc(n.name)} ${marks}</td><td class="n">${n.rank ?? DASH}</td><td class="n">${signed(n.relation, 0)}</td><td>${dipLevel(n.opinion)}</td>${gp}</tr>`;
  }).join('');
  return filters + part('diplomacy.table', `<div class="dtable"><table class="tbl">${head}${rows || `<tr><td colspan="${5 + powers.length}" class="small">No country here.</td></tr>`}</table></div>`);
}

// ---- the selected country and its actions
function dipSelected(d, all, played, t) {
  if (!t) return part('diplomacy.selected', `<div class="row dsel"><div class="rowbody small">Pick a country in the table or a great power above.</div></div>`);
  const isGp = (d.powers || []).find(p => p.tag === t.tag);
  // Opinion between the two: the seat's of them when the seat is a great power, else theirs of the seat when they are one.
  const op = d.seat_great_power ? t.opinion : isGp ? isGp.opinion : null;
  const chip = (on, label, cls = '') => `<span class="dchip ${on ? 'on ' + cls : ''}">${label}</span>`;
  const flags = `${chip(has(op) && op >= 3, 'Cordial')}${chip(has(op) && op >= 4, 'Friendly')}${chip(!!t.sphere_of, t.sphere_of ? 'Sphere of ' + esc(dipName(all, d, t.sphere_of)) : 'Sphere', 'sph')}${chip(t.allied, 'Ally', 'good')}${chip(t.at_war, 'At war', 'bad')}${chip(t.truce, 'Truce')}${chip(t.access, 'Access')}${chip(t.guaranteed, 'Guarantee')}${t.overlord ? chip(true, 'Subject of ' + esc(dipName(all, d, t.overlord))) : ''}`;
  return part('diplomacy.selected', `<div class="row dsel">
    <div class="dshead">${flagTile(t.tag, t.colour, { rank: t.rank, size: 'lg', title: t.name })}<div><b class="big">${esc(t.name)}</b><div class="small">${esc(DIP_STATUS[t.status] || '')}${t.rank ? `, rank ${t.rank}` : ''}</div><div class="small">${esc(t.government || '')}</div></div></div>
    <div class="kv">
      ${part('diplomacy.selected.prestige', `<div><span>${ic('prestige')} Prestige</span><span>${num(t.prestige)}</span></div>`)}
      ${part('diplomacy.selected.industry', `<div><span>${ic('industry')} Industry</span><span>${num(t.industry)}</span></div>`)}
      ${part('diplomacy.selected.military', `<div><span>${ic('military')} Military</span><span>${num(t.military)}</span></div>`)}
      ${part('diplomacy.selected.total', `<div><span>Total score</span><span>${num(t.total)}</span></div>`)}
      ${part('diplomacy.selected.population', `<div><span>Population</span><span>${num(t.population)}</span></div>`)}
      ${part('diplomacy.selected.relation', `<div><span>Relation</span><span>${signed(t.relation, 0)}</span></div>`)}
      ${part('diplomacy.selected.infamy', `<div><span>${ic('infamy')} Infamy</span><span>${num(t.infamy, 1)}</span></div>`)}
      ${part('diplomacy.selected.opinion', `<div><span>Opinion</span><span>${dipLevel(op)}</span></div>`)}
    </div>
    ${part('diplomacy.selected.cultures', `<div class="chips">${(t.cultures || []).map((c, i) => `<span title="${i ? 'accepted' : 'primary'} culture">${i ? '' : '<b>'}${esc(c)}${i ? '' : '</b>'}</span>`).join('') || `<span>${DASH}</span>`}</div>`)}
    ${part('diplomacy.selected.flags', `<div class="dchips">${flags}</div>`)}
  </div>`);
}

function dipActions(d, all, played, t) {
  const j = d.justifying, cbs = d.cbs || [], held = t?.held || [];
  const seatWar = t && (d.wars || []).some(w => (w.talk || []).includes(t.tag));
  const reason = !t ? 'Pick a country first.' : !played ? 'Play this country to act.' : '';
  const live = (label, ok, why, attrs) => ok ? btn(label, 'sm', attrs) : btn(label, 'sm off', `disabled title="${esc(why)}"`);
  const notYet = label => btn(label, 'sm off', 'disabled title="not yet"');
  const declare = live('Declare war', !reason && held.length && !t.at_war && !t.allied, reason || (t.at_war ? 'Already at war.' : t.allied ? 'An ally.' : 'No casus belli on them: justify one first.'), `data-declare="${esc(held[0]?.cb || '')}"`);
  const justify = live('Justify war', !reason && !j && !t.at_war && !t.truce && cbs.length, reason || (j ? 'One justification at a time.' : t.at_war ? 'Already at war.' : t.truce ? 'A truce holds.' : 'No casus belli can be justified.'), 'data-dip-tab="justify"');
  const peace = live('Offer peace', !reason && seatWar, reason || 'Not at war with them, or they do not lead their side.', 'data-dip-tab="wars"');
  const grid = [
    notYet('Form alliance'), notYet('Cancel alliance'), notYet('Call ally'),
    notYet('Ask military access'), notYet('Cancel access'), notYet('War subsidies'),
    declare, peace, notYet('Command units'),
    justify, notYet('Guarantee independence'), notYet('Discredit'),
    notYet('Increase opinion'), notYet('Decrease opinion'), notYet('Expel advisors'),
    notYet('Add to sphere'), notYet('Remove from sphere'), notYet('Ban embassy'),
  ];
  return part('diplomacy.actions', `<div class="dacts">${grid.join('')}</div>`);
}

WIN.diplomacy = S => {
  const d = DIPLO, played = !!S.country?.played;
  if (!d) return frameWin('Diplomacy', `<div class="todo">Asking the game&hellip;</div>`);
  const all = dipNations(d), target = all[DIPSEL.target] || null;
  const note = played ? '' : `<p class="small" style="margin:.2rem .4rem">The computer plays this country and makes its wars and peace. Play it to do so yourself.</p>`;
  const questions = (d.questions || []).length ? part('diplomacy.questions', band('Waiting for your answer') + d.questions.map(q => questionHtml(q, played)).join('')) : '';
  const nWars = (d.all_wars || d.wars || []).length;
  const tabs = [['powers', 'Great Powers'], ['wars', `Wars${nWars ? ` (${nWars})` : ''}`], ['justify', 'War Justification'], ['crisis', 'Crisis']];
  const tabBar = part('diplomacy.tabs', `<div class="tabs dtabs" role="tablist">${tabs.map(([k, l]) => `<span role="tab" tabindex="0" aria-selected="${DIPSEL.tab === k}" class="${DIPSEL.tab === k ? 'on' : ''}" data-dip-tab="${k}">${l}</span>`).join('')}</div>`);
  const body = DIPSEL.tab === 'wars' ? dipWars(d, all, played) : DIPSEL.tab === 'justify' ? dipJustify(d, all, played, target) : DIPSEL.tab === 'crisis' ? dipCrisis(d) : dipPowers(d, all);
  return frameWin('Diplomacy', `${note}${questions}<div class="dip">${tabBar}<div class="dtop">${body}</div>
    <div class="dmain"><div class="dleft">${dipTable(d, all)}</div><div class="dright">${dipSelected(d, all, played, target)}${dipActions(d, all, played, target)}</div></div></div>`);
};

// The window's own choices: tab, filter, sort, the selected country.
document.addEventListener('click', e => {
  const b = e.target.closest('[data-dip-tab],[data-dip-filter],[data-dip-sort],[data-dip-pick]');
  if (!b || b.disabled) return;
  if (b.dataset.dipTab) DIPSEL.tab = b.dataset.dipTab;
  else if (b.dataset.dipFilter) DIPSEL.filter = b.dataset.dipFilter;
  else if (b.dataset.dipSort) { const k = b.dataset.dipSort; DIPSEL.dir = DIPSEL.sort === k ? -DIPSEL.dir : (k === 'name' || k === 'rank' ? 1 : -1); DIPSEL.sort = k; }
  else if (b.dataset.dipPick) DIPSEL.target = b.dataset.dipPick;
  drawAll();
});
document.addEventListener('keydown', e => { const t = e.target; if (e.key === 'Enter' && t.matches?.('[data-dip-tab]')) { e.preventDefault(); t.click(); } });
