// POPULATION. One window = one file. It reads POPS (GET /api/population, asked here while this
// window is open; js/live.js calls refreshPopulation on its round). Left: the seat's states and
// provinces, a tree that filters everything else. Right: pies of the chosen pops (types, culture,
// religion, ideology, issues), one toggle per pop type, and a sortable, paged table of the pops.
// POPSEL keeps what the player chose; the game sorts and pages, so only one page travels.
let POPS = null;
const POPSEL = { state: null, province: null, types: [], sort: 'size', desc: true, offset: 0, limit: 25, open: new Set() };
let popsAsked = 0, popsKey = '';

function popsQuery() {
  const s = POPSEL;
  return `state=${s.state ?? ''}&province=${s.province ?? ''}&type=${encodeURIComponent(s.types.join(','))}&sort=${s.sort}&desc=${s.desc}&offset=${s.offset}&limit=${s.limit}`;
}
async function refreshPopulation(force = false) {
  if (open !== 'population') { POPS = null; return; }
  const q = popsQuery();
  if (!force && q === popsKey && Date.now() - popsAsked < 2000) return;
  popsAsked = Date.now(); popsKey = q;
  try { POPS = await ask(`/api/population?${q}`); } catch (e) { /* the next round tries again */ }
}
async function popsChange(f) { f(POPSEL); await refreshPopulation(true); drawAll(); }

// A pop type's figure: the kit's pop figure, tinted with the type's colour from the install.
// a pop type's icon where one is drawn (by its key), else a small figure in the type's colour
const popFig = t => hasIcon(t?.key) ? `<span class="popic" title="${esc(t?.name || '')}">${ic(t.key)}</span>` : `<i class="popfig" style="--c:${esc(t?.colour || '')}" title="${esc(t?.name || '')}"></i>`;

// A pie with its largest slices named underneath.
function pieBox(name, title, parts) {
  const legend = (parts || []).slice(0, 4).map(([n, s], i) => `<li><i style="background:${PIE_COLOURS[i]}"></i><span>${esc(n)}</span><b>${pct(s)}</b></li>`).join('');
  return part(`population.summary.${name}`, `<figure class="ppie"><figcaption>${title}</figcaption>${pie(parts)}<ul>${legend || '<li class="small">none</li>'}</ul></figure>`);
}

WIN.population = S => {
  const d = POPS, played = !!S.country?.played;
  if (!d) return frameWin('Population', `<div class="todo">Asking the game&hellip;</div>`);
  if (!d.exists) return frameWin('Population', `<div class="todo">This country has no land and no people.</div>`);
  const sel = POPSEL, sum = d.summary || {}, pg = d.pops || { rows: [], count: 0 };
  const typeOf = Object.fromEntries((d.types || []).map(t => [t.key, t]));
  const stateOf = Object.fromEntries((d.states || []).map(s => [s.id, s]));

  // ---- left: the tree of states and provinces
  const node = (label, pop, attrs, on, extra = '') => `<button class="pnode ${on ? 'on' : ''}" ${attrs}><span>${label}</span>${extra}<b>${num(pop)}</b></button>`;
  const tree = (d.states || []).map(s => {
    const isOpen = sel.open.has(s.id), on = sel.state === s.id && sel.province == null;
    const focus = s.focus ? `<em title="National focus">${ic('focus')}${esc(s.focus)}</em>` : '';
    const provs = isOpen ? `<div class="pkids">${s.provinces.map(p => node(esc(p.name), p.population, `data-pop-province="${p.id}" data-state="${s.id ?? ''}"`, sel.province === p.id)).join('')}</div>` : '';
    return `<div class="pstate"><button class="ptwist" data-pop-twist="${s.id ?? ''}" aria-expanded="${isOpen}" aria-label="Show the provinces of ${esc(s.name)}">${isOpen ? '&#9662;' : '&#9656;'}</button>${node(esc(s.name), s.population, `data-pop-state="${s.id ?? ''}"`, on, focus)}</div>${provs}`;
  }).join('');
  const all = node(`<b>${esc(S.country?.name || d.tag)}</b>`, d.population, 'data-pop-all', sel.state == null && sel.province == null);
  const focus = part('population.focus', `<div class="row"><span class="rowhead">${ic('focus')} National focus</span><div class="rowbody"><span class="value">${has(d.focus_used) ? d.focus_used : DASH} / ${has(d.focus_max) ? d.focus_max : DASH}</span>${btn('Set focus', 'sm', 'disabled title="not yet"')}</div></div>`);
  const left = `<div class="pleft">${focus}${part('population.tree', band('States') + `<div class="ptree">${all}${tree}</div>`)}</div>`;

  // ---- right: where we look, the pies, the type toggles, the table
  const where = sel.province != null ? (stateOf[sel.state]?.provinces || []).find(p => p.id === sel.province)?.name || 'Province'
    : sel.state != null ? stateOf[sel.state]?.name || 'State' : S.country?.name || d.tag;
  const facts = part('population.summary.facts', `<div class="pfacts">
      <span title="People">${ic('pops')}<b>${num(sum.population)}</b></span><span title="Pops">${num(sum.pops)} pops</span>
      <span title="Literacy">${ic('literacy')}${pct(sum.literacy, 1)}</span><span title="Militancy">${ic('militancy')}${num(sum.militancy, 2)}</span>
      <span title="Consciousness">${ic('consciousness')}${num(sum.consciousness, 2)}</span><span title="Out of work">Unemployed ${pct(sum.unemployment, 1)}</span>
      <span title="Cash held by these pops">${ic('money')}${money(sum.cash, 0)}</span></div>`);
  const pies = `<div class="ppies">${pieBox('types', 'Pop types', sum.types)}${pieBox('culture', 'Culture', sum.culture)}${pieBox('religion', 'Religion', sum.religion)}${pieBox('ideology', 'Ideology', sum.ideology)}${pieBox('issues', 'Issues', sum.issues)}</div>`;
  const toggles = part('population.types', `<div class="ptypes">${btn('All', 'sm ptall', `data-pop-type="" aria-pressed="${!sel.types.length}"`)}${(d.types || []).map(t =>
    `<button class="ptype" data-pop-type="${esc(t.key)}" aria-pressed="${!sel.types.length || sel.types.includes(t.key)}" title="${esc(t.name)}: ${num(t.population)} (${esc(t.strata)} strata)">${popFig(t)}<span>${num(t.population)}</span></button>`).join('')}</div>`);

  const cols = [['type', 'Type'], ['size', 'Size'], ['culture', 'Culture'], ['religion', 'Religion'], ['location', 'Location'],
    ['militancy', ic('militancy'), 'Militancy'], ['consciousness', ic('consciousness'), 'Consciousness'], ['literacy', ic('literacy'), 'Literacy'],
    ['unemployment', 'Unempl.', 'Out of work'], ['cash', ic('money'), 'Cash'], ['needs', 'Needs', 'Life, everyday and luxury needs met']];
  const head = cols.map(([k, label, title]) => `<th><button class="psort ${pg.sort === k ? 'on' : ''}" data-pop-sort="${k}" title="${esc(title || '')} (sort)">${label}${pg.sort === k ? (pg.desc ? ' &#9662;' : ' &#9652;') : ''}</button></th>`).join('');
  const need = (v, what) => `<span class="pneed" title="${what} needs ${pct(v)}"><b style="width:${has(v) ? Math.round(Math.min(1, v) * 100) : 0}%"></b></span>`;
  const rows = pg.rows.map(r => `<tr>
      <td>${popFig(typeOf[r.type] || { key: r.type, name: r.type_name })} ${esc(r.type_name)}</td><td class="n">${num(r.size)}</td><td>${esc(r.culture)}</td><td>${esc(r.religion)}</td>
      <td>${esc(r.province?.name)}<span class="small"> &middot; ${esc(stateOf[r.state]?.name || '')}</span></td>
      <td class="n ${r.militancy >= 5 ? 'bad' : ''}">${num(r.militancy, 2)}</td><td class="n">${num(r.consciousness, 2)}</td><td class="n">${pct(r.literacy, 1)}</td>
      <td class="n ${r.unemployment > 0 ? 'bad' : ''}">${pct(r.unemployment, 1)}</td><td class="n">${money(r.cash, 2)}</td>
      <td class="pneeds">${need(r.needs?.life, 'Life')}${need(r.needs?.everyday, 'Everyday')}${need(r.needs?.luxury, 'Luxury')}</td></tr>`).join('');
  const pages = Math.max(1, Math.ceil(pg.count / pg.limit)), page = Math.floor(pg.offset / pg.limit) + 1;
  const pager = part('population.pops.pager', `<div class="ppager">${btn('Previous', 'sm', `data-pop-page="-1" ${page <= 1 ? 'disabled' : ''}`)}<span class="pbadge">${page}</span><span class="small">of ${pages} &middot; ${num(pg.count)} pops</span>${btn('Next', 'sm', `data-pop-page="1" ${page >= pages ? 'disabled' : ''}`)}</div>`);
  const table = part('population.pops.table', `<table class="tbl ptbl"><tr>${head}</tr>${rows || `<tr><td colspan="${cols.length}" class="small">No pops of these types here.</td></tr>`}</table>`);
  const note = played ? '' : `<span class="small">The computer plays this country.</span>`;
  const right = `<div class="pright">${part('population.summary', band(`Population of ${esc(where)}`) + `<div class="row psum">${facts}${pies}</div>`)}${toggles}${table}${pager}${note}</div>`;
  return frameWin('Population', `<div class="popwin">${left}${right}</div>`);
};

// The window's own clicks (main.js handles the frame and the other windows).
document.addEventListener('click', e => {
  const on = sel => e.target.closest(sel);
  let b;
  if (!on('.popwin')) return;
  if (on('[data-pop-all]')) return popsChange(s => { s.state = null; s.province = null; s.offset = 0; });
  if ((b = on('[data-pop-twist]'))) { const id = b.dataset.popTwist === '' ? null : +b.dataset.popTwist; POPSEL.open.has(id) ? POPSEL.open.delete(id) : POPSEL.open.add(id); return drawAll(); }
  if ((b = on('[data-pop-state]'))) return popsChange(s => { s.state = b.dataset.popState === '' ? null : +b.dataset.popState; s.province = null; s.offset = 0; s.open.add(s.state); });
  if ((b = on('[data-pop-province]'))) return popsChange(s => { s.province = +b.dataset.popProvince; s.state = b.dataset.state === '' ? null : +b.dataset.state; s.offset = 0; });
  if ((b = on('[data-pop-type]'))) return popsChange(s => {
    const k = b.dataset.popType, keys = (POPS?.types || []).map(t => t.key);
    if (!k) s.types = [];
    else {
      const shown = new Set(s.types.length ? s.types : keys);
      shown.has(k) ? shown.delete(k) : shown.add(k);
      s.types = shown.size === keys.length ? [] : shown.size ? [...shown] : [k];
    }
    s.offset = 0;
  });
  if ((b = on('[data-pop-sort]'))) return popsChange(s => { const k = b.dataset.popSort; s.desc = s.sort === k ? !s.desc : !['type', 'culture', 'religion', 'location'].includes(k); s.sort = k; s.offset = 0; });
  if ((b = on('[data-pop-page]'))) return popsChange(s => { s.offset = Math.max(0, (POPS?.pops?.offset ?? s.offset) + +b.dataset.popPage * s.limit); });
});
