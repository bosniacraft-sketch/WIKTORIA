// THE KIT IN CODE. Small helpers every screen is built from. A helper returns a piece of markup; what
// it looks like is decided in style/kit.css. Nothing here knows about the game.
const $ = id => document.getElementById(id);
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

// Icons: a file from icons/ when icons/index.json lists the id (the owner's replacements), else our
// drawing (js/icons.js, by id or through ICON_ALIAS), else a brass token with two letters.
const iconById = Object.fromEntries(ICONS.map(i => [i.id, i]));
const ICON_FILES = new Set();
fetch('icons/index.json').then(r => r.ok ? r.json() : []).then(ids => { for (const x of ids || []) if (/^[a-z0-9_]+$/.test(x)) ICON_FILES.add(x); }).catch(() => {});
const iconOf = id => [id, ICON_ALIAS[id]].find(k => k && (ICON_FILES.has(k) || iconById[k])) || null;
const hasIcon = id => !!iconOf(id);
const iconCache = {};
const ic = id => {
  const k = iconOf(id);
  if (!k) return `<span class="ph" title="${esc(id)} (icon not drawn yet)">${esc(String(id).slice(0, 2))}</span>`;
  return `<span class="ic">${ICON_FILES.has(k) ? `<img src="icons/${k}.svg" alt="">` : iconCache[k] ||= svgOf(iconById[k], 24)}</span>`;
};
const it = (id, text, title = '') => `<span class="it" title="${esc(title)}">${ic(id)}${text}</span>`;

// Numbers as the player reads them. A value the game does not have yet is null: shown as a dash.
const DASH = '&ndash;';
const has = v => v !== null && v !== undefined && Number.isFinite(+v);
function num(v, digits = 0) { if (!has(v)) return DASH; const a = Math.abs(v); return a >= 1e6 ? (v / 1e6).toFixed(2) + 'M' : a >= 1e4 ? (v / 1e3).toFixed(1) + 'k' : (+v).toFixed(digits); }
const money = (v, digits = 1) => has(v) ? `${num(v, digits)} &pound;` : DASH;
const pct = (v, digits = 0) => has(v) ? (v * 100).toFixed(digits) + '%' : DASH;
const signed = (v, digits = 1) => has(v) ? `<span class="${v < 0 ? 'bad' : 'good'}">${v < 0 ? '&minus;' : '+'}${num(Math.abs(v), digits)}</span>` : DASH;

// alertChip(letters, colour, label, id): the alert's icon when one is drawn (id), else its letters.
const alertChip = (letters, colour, label, id = '') => `<span class="alert${hasIcon(id) ? ' alic' : ''}" style="--c:${colour}" title="${esc(label)}">${hasIcon(id) ? ic(id) : letters}</span>`;
// slider(share): shows a share 0..1. With `live` = {name, lo, hi} (shares too) it can be moved: the
// arrows and a click on the track send the new value (js/main.js), and the allowed part is marked.
const slider = (share, live = null) => {
  const at = has(share) ? Math.round(share * 100) : 50;
  if (!live) return `<span class="slider"><i>&#9664;</i><span class="track"><b style="--at:${at}%"></b></span><i>&#9654;</i></span>`;
  const lo = Math.round((live.lo ?? 0) * 100), hi = Math.round((live.hi ?? 1) * 100);
  return `<span class="slider live" data-slider="${esc(live.name)}" data-at="${has(share) ? share : ''}" title="${esc(live.title || '')}"><i data-step="-1">&#9664;</i><span class="track"><s style="--lo:${lo}%;--hi:${hi}%"></s><b style="--at:${at}%"></b></span><i data-step="1">&#9654;</i></span>`;
};
const PIE_COLOURS = ['#8c2a22', '#c9a23a', '#4f6a8a', '#6f8a3a', '#8a6f52', '#7a5a8a', '#3a8a8a', '#9a9a9a'];
// pie([["farmers", 0.62], ...]): slices in the order given; the names go into the tooltip.
function pie(parts) {
  if (!parts || !parts.length) return `<span class="pie" style="background:#d8d4c6" title="no data"></span>`;
  let at = 0; const stops = parts.slice(0, 8).map(([, share], i) => { const from = at; at += share * 100; return `${PIE_COLOURS[i]} ${from.toFixed(1)}% ${at.toFixed(1)}%`; });
  if (at < 99.5) stops.push(`#b8b4a6 ${at.toFixed(1)}% 100%`);
  return `<span class="pie" style="background:conic-gradient(${stops.join(',')})" title="${esc(parts.slice(0, 8).map(([n, s]) => `${n} ${(s * 100).toFixed(0)}%`).join(', '))}"></span>`;
}
const pops = n => `<span class="pops">${'<i></i>'.repeat(n)}</span>`;
const btn = (label, cls = '', attrs = '') => `<button class="btn ${cls}" ${attrs}><span>${label}</span></button>`;
const band = t => `<div class="band">${t}</div>`;
// part('window.panel.row', markup): gives a part our own name, so tools can find it and a mod can address it.
const part = (name, html) => `<div data-n="${name}" style="display:contents">${html}</div>`;

const WIN = {};    // window name -> function(S) that returns its markup (see js/windows/)
function frameWin(title, body) { return `<div class="wback"><span class="wtitle">${title}</span><button class="close wclose" data-close aria-label="Close ${title}">&times;</button><div class="wwin paper frame">${body}</div></div>`; }
const notBuilt = title => frameWin(title, `<div class="todo"><b>Not laid out yet.</b> This window has its frame and its place on the top bar; its contents come next.</div>`);

// Replaces an element's contents only when they changed, so hovering and focus survive the refresh.
const lastHtml = new WeakMap();
function paint(el, html) { if (lastHtml.get(el) !== html) { el.innerHTML = html; lastHtml.set(el, html); } }

// ---- Diplomacy: a country's flag tile, drawn from its map colour (our own pattern, picked by the
// tag so a country keeps its tile): flagTile(tag, '#rrggbb', {rank, size: 'sm'|'md'|'lg'}).
function flagTile(tag, colour, o = {}) {
  let h = 0; for (const ch of String(tag || '')) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  const rank = has(o.rank) ? `<span class="fr">${o.rank}</span>` : '';
  return `<span class="ftile p${h % 6} ${o.size || 'md'}" style="--c:${esc(colour || '#808080')}" title="${esc(o.title ?? tag ?? '')}">${rank}</span>`;
}
// ---- added for the Politics window
// pieKey(["first", ...]): the names of a pie's slices with their colours, in the order pie() gives them.
const pieKey = names => `<div class="piekey">${names.slice(0, 8).map((n, i) => `<span><i style="background:${PIE_COLOURS[i]}"></i>${esc(n)}</span>`).join('')}</div>`;
// ---- Trade window (js/windows/trade.js), reusable by any ledger page.
// trend(now, before): an arrow up, down or level, coloured by style/kit.css; a dash without both.
const trend = (now, before, label = '') => {
  if (!has(now) || !has(before)) return `<span class="trend" title="${esc(label)}">${DASH}</span>`;
  const d = now - before, eps = Math.max(1e-6, Math.abs(before) * 1e-4);
  const [cls, ch] = d > eps ? ['up', '&#9650;'] : d < -eps ? ['down', '&#9660;'] : ['flat', '&#9679;'];
  return `<span class="trend ${cls}" title="${esc(label)}${label ? ': ' : ''}${d >= 0 ? '+' : '&minus;'}${num(Math.abs(d), 2)}">${ch}</span>`;
};
// lineChart(values, {w, h, base, from}): a line drawn as inline SVG, one point per value, oldest
// first; `base` draws a dashed level line (the good's base price). Colours come from kit.css.
function lineChart(values, o = {}) {
  const w = o.w || 400, h = o.h || 120, pad = 4, v = (values || []).filter(has);
  if (v.length < 2) return `<svg class="chart" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="no history yet"><text x="${w / 2}" y="${h / 2}" text-anchor="middle">${o.empty || 'No history yet'}</text></svg>`;
  let lo = Math.min(...v, has(o.base) ? o.base : Infinity), hi = Math.max(...v, has(o.base) ? o.base : -Infinity);
  if (hi - lo < 1e-9) { lo -= 1; hi += 1; }
  const x = i => pad + (w - 2 * pad) * i / (v.length - 1), y = p => h - pad - (h - 2 * pad) * (p - lo) / (hi - lo);
  const pts = v.map((p, i) => `${x(i).toFixed(1)},${y(p).toFixed(1)}`).join(' ');
  const grid = [0.25, 0.5, 0.75].map(f => `<line class="grid" x1="0" x2="${w}" y1="${(pad + (h - 2 * pad) * f).toFixed(1)}" y2="${(pad + (h - 2 * pad) * f).toFixed(1)}"/>`).join('');
  const base = has(o.base) ? `<line class="base" x1="0" x2="${w}" y1="${y(o.base).toFixed(1)}" y2="${y(o.base).toFixed(1)}"/>` : '';
  return `<svg class="chart" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="${esc(o.label || 'history')}">${grid}${base}<polygon class="area" points="${x(0).toFixed(1)},${h} ${pts} ${x(v.length - 1).toFixed(1)},${h}"/><polyline class="line" points="${pts}"/></svg>
    <div class="chartaxis"><span>${esc(o.from || '')}</span><span>low ${num(Math.min(...v), 2)} &middot; high ${num(Math.max(...v), 2)}</span><span>${esc(o.to || '')}</span></div>`;
}
// ---- added for PRODUCTION (js/windows/production.js); usable by any window
// tabStrip([[id, label], ...], on, attr): a row of tabs; each carries attr="id" for a click handler.
const tabStrip = (items, on, attr) => `<div class="tabs" role="tablist">${items.map(([id, label]) => `<span role="tab" tabindex="0" aria-selected="${id === on}" class="${id === on ? 'on' : ''}" ${attr}="${esc(id)}">${label}</span>`).join('')}</div>`;
// goodTile(key, name, colour): the good's own icon where one is drawn, else a roundel in the
// good's colour from the goods file with two letters of its name.
const goodTile = (key, name, colour) => hasIcon(key) ? `<span class="gtile" title="${esc(name || key)}">${ic(key)}</span>`
  : `<span class="gtile tok" style="--g:${esc(colour || '#9a8a6a')}" title="${esc(name || key)}">${esc(String(name || key).replace(/[^A-Za-z]/g, '').slice(0, 2))}</span>`;
// meter(v, max): a thin bar of v out of max with the numbers in its title.
const meter = (v, max, title = '') => `<span class="bar meter" title="${esc(title)}"><b style="width:${has(v) && has(max) && max > 0 ? Math.min(100, Math.round(v / max * 100)) : 0}%"></b></span>`;
