// POLITICS. One window = one file. It reads POLITICS (GET /api/politics, asked by js/live.js while
// this window is open) and, for the decisions tab, EVENTS (GET /api/events). Left: the government,
// its parties and the three ideology pies, the important issues. Right: tabs for reforms,
// movements and rebels, decisions and release nations. Only decisions are live orders (POST
// /api/decision, js/main.js); reforms and releasing a country have no order in the engine yet, so
// their buttons are drawn disabled with "not yet".
const POLSEL = { tab: 'reforms' };
const POL_TABS = [['reforms', 'Reforms'], ['movements', 'Movements & Rebels'], ['decisions', 'Decisions'], ['release', 'Release Nations']];
let politicsAsking = false;

document.addEventListener('click', e => {
  const b = e.target.closest('[data-pol-tab]');
  if (b) { POLSEL.tab = b.dataset.polTab; drawAll(); }
});

WIN.politics = S => {
  const p = POLITICS, played = !!S.country?.played;
  if (!p) {
    if (!politicsAsking) { politicsAsking = true; refreshPolitics(true).finally(() => { politicsAsking = false; drawAll(); }); }
    return frameWin('Politics', `<div class="todo">Asking the game&hellip;</div>`);
  }
  const notYet = `disabled title="not yet"`;
  const ideoName = Object.fromEntries((p.ideologies || []).map(i => [i.key, i.name]));
  const slices = list => list ? list.map(([k, v]) => [ideoName[k] || k, v]) : null;
  const pieBox = (name, title, list) => part(`politics.government.${name}`, `<figure class="polpie">${pie(slices(list))}<figcaption>${title}</figcaption></figure>`);
  const dateOr = (d, none) => d ? esc(prettyDate(d)) : none;
  const ymd = d => (d || '').split('.').reduce((a, x) => a * 100 + +x, 0);

  // ---- left: the government
  const g = p.government, rp = p.ruling_party;
  const elections = g?.elections === false ? 'This government holds no elections.' : `Next election ${dateOr(p.next_election, DASH)}`;
  const head = part('politics.government.head', `<div class="row"><span class="rowhead">${esc(g?.name || DASH)}</span>
    <div class="rowbody"><span class="polparty"><b>${esc(rp?.name || DASH)}</b>${rp?.ideology_name ? ` <span class="small">(${esc(rp.ideology_name)})</span>` : ''}</span>
    <span class="small">${elections}</span></div>
    <div class="rowbody small"><span>National value: <b>${esc(p.national_value || DASH)}</b></span><span>Militancy ${num(p.militancy, 2)}</span><span>Consciousness ${num(p.consciousness, 2)}</span></div></div>`);
  const ruling = (p.parties || []).find(x => x.ruling);
  const policies = part('politics.government.policies', band('Party Policies') + `<div class="kv polkv">${(ruling?.policies || []).map(x => `<div><span>${esc(x.group_name || DASH)}</span><span>${esc(x.option_name)}</span></div>`).join('') || `<p class="small">${DASH}</p>`}</div>`);
  const pies = `<div class="row polpies">${pieBox('upper_house', 'Upper House', p.upper_house)}${pieBox('voters', 'Voters&rsquo; Ideology', p.voters)}${pieBox('people', 'People&rsquo;s Ideology', p.people)}</div>`
    + part('politics.government.key', pieKey((p.ideologies || []).map(i => i.name)));
  const parties = part('politics.government.parties', `<div class="small polparties">${(p.parties || []).map(x => `<span class="${x.ruling ? 'on' : ''} ${x.allowed ? '' : 'off'}" title="${x.allowed ? '' : 'not allowed under this government'}">${esc(x.name || DASH)} <i>(${esc(x.ideology_name)})</i></span>`).join('')}</div>`);
  const issues = part('politics.government.issues', band('Important Issues') + ((p.issues || []).length
    ? `<table class="tbl"><tr><th>Issue</th><th>Group</th><th>Share</th></tr>${p.issues.map(i => `<tr><td>${esc(i.name)}</td><td class="small">${esc(i.group_name || '')}</td><td class="n">${pct(i.share, 1)}</td></tr>`).join('')}</table>`
    : `<p class="small pad">${DASH}</p>`));
  const left = `<div class="col">${part('politics.government.header', band('Government'))}${head}${pies}${parties}${policies}${issues}</div>`;

  // ---- right: the tabs
  const tabs = `<div class="tabs poltabs">${POL_TABS.map(([id, label]) => `<button data-pol-tab="${id}"><span class="${POLSEL.tab === id ? 'on' : ''}">${label}</span></button>`).join('')}</div>`;
  let body = '';
  if (POLSEL.tab === 'reforms') {
    const r = p.reforms || {};
    const support = part('politics.reforms.support', `<div class="rowbody small polsupport"><span>Upper house for political reform <b>${pct(r.house_political)}</b></span><span>for social reform <b>${pct(r.house_social)}</b></span><span>needed ${pct(r.support_needed)}</span><span>Next reform ${r.next_possible && ymd(r.next_possible) > ymd(p.date) ? 'from ' + dateOr(r.next_possible) : 'possible now'}</span></div>`);
    const group = x => part(`politics.reforms.${x.key}`, `<div class="row polgroup"><span class="rowhead">${esc(x.name)}</span><div class="rowbody">${x.options.map(o =>
      `<button class="polopt ${o.current ? 'cur' : ''} ${o.open ? 'open' : ''}" ${notYet}>${esc(o.name)}<small>${pct(o.wanted)}</small></button>`).join('')}</div></div>`);
    const kind = political => (r.groups || []).filter(x => x.political === political).map(group).join('') || `<p class="small pad">${DASH}</p>`;
    const note = played ? 'Passing a reform yourself is not in the game yet: the upper house passes them.' : 'The computer plays this country and its upper house passes its reforms.';
    body = `${support}<p class="small pad">${note} Dark: in force. Dashed: may be passed next. The share is of people asking for it.</p>
      <div class="polreforms"><div>${band('Political Reforms')}${kind(true)}</div><div>${band('Social Reforms')}${kind(false)}</div></div>`;
  } else if (POLSEL.tab === 'movements') {
    const mv = p.movements || [], rb = p.rebels || [], ra = p.rebel_armies || {};
    body = part('politics.movements.list', band('Movements') + (mv.length
      ? `<table class="tbl"><tr><th>Movement</th><th>For</th><th>Support</th><th>Radicalism</th></tr>${mv.map(m => `<tr><td>${esc(m.target_name)}</td><td class="small">${m.kind === 'independence' ? 'independence' : 'reform'}</td><td class="n">${num(m.support)}</td><td class="n">${num(m.radicalism, 1)}</td></tr>`).join('')}</table>`
      : `<p class="small pad">No movement in this country.</p>`))
      + part('politics.rebels.list', band('Rebels') + (rb.length
      ? `<table class="tbl"><tr><th>Faction</th><th>Type</th><th>Organisation</th><th>People</th></tr>${rb.map(f => `<tr><td>${esc(f.name)}</td><td class="small">${esc(f.kind_name)}</td><td><span class="bar polbar"><b style="width:${Math.round((f.organisation || 0) * 100)}%"></b></span> ${pct(f.organisation)}</td><td class="n">${num(f.people)}</td></tr>`).join('')}</table>`
      : `<p class="small pad">No rebel faction in this country.</p>`))
      + part('politics.rebels.armies', `<p class="small pad">Rebel armies on its land: <b>${num(ra.armies)}</b> (${num(ra.regiments)} regiments).</p>`);
  } else if (POLSEL.tab === 'decisions') {
    const list = EVENTS?.decisions || [];
    body = !played ? `<p class="small pad">The computer plays this country and takes its decisions. Play it to take them yourself.</p>`
      : !EVENTS ? `<div class="todo">Asking the game&hellip;</div>`
      : part('politics.decisions.list', list.length ? list.map(d => `<div class="row poldecision"><div class="rowbody"><span class="poldesc"><b>${esc(d.title)}</b>${d.desc ? `<span class="small">${esc(d.desc)}</span>` : ''}</span>
          ${btn(d.allowed ? 'Enact' : 'Not yet', 'sm', `data-decide="${esc(d.key)}" ${d.allowed ? '' : 'disabled title="its conditions do not hold yet"'}`)}</div></div>`).join('')
        : `<p class="small pad">No decision is open to this country.</p>`);
  } else {
    const rel = p.release || [];
    body = part('politics.release.list', `<p class="small pad">Countries that own no land and have cores on this country&rsquo;s provinces. Releasing one is not in the game yet.</p>` + (rel.length
      ? `<table class="tbl"><tr><th>Country</th><th>Provinces</th><th></th></tr>${rel.map(x => `<tr><td>${esc(x.name)}</td><td class="n">${num(x.provinces)}</td><td>${btn('Release', 'sm', notYet)}</td></tr>`).join('')}</table>`
      : `<p class="small pad">No country to release.</p>`));
  }
  const right = `<div class="col">${part('politics.tabs', tabs)}<div class="poltab">${body}</div></div>`;
  return frameWin('Politics', `<div class="cols polcols">${left}${right}</div>`);
};
