// TECHNOLOGY. One window = one file. It reads RESEARCH (GET /api/research, asked by js/live.js while
// this window is open): what is being researched, and what may be researched next, by folder.
// While the seat is played each open technology has a button that sends POST /api/research.
WIN.technology = S => {
  const r = RESEARCH, played = !!S.country?.played;
  if (!r) return frameWin('Technology', `<div class="todo">Asking the game&hellip;</div>`);
  const cur = r.current;
  const now = part('technology.current', `<div class="row"><span class="rowhead">Researching</span><div class="rowbody">
      <b style="flex:1">${esc(cur?.name || 'Nothing')}</b>${has(cur?.progress) ? `<span class="bar" style="max-width:10rem"><b style="width:${Math.round(cur.progress * 100)}%"></b></span><span>${pct(cur.progress)}</span>` : ''}
      ${it('research', num(r.points_a_day, 1), 'Research points a day')}<span class="small">stored ${num(r.points_stored, 0)}</span></div></div>`);
  const note = !r.civilized ? 'An uncivilized country does not research.'
    : played ? (cur ? 'Choose another technology to switch; the points already spent are kept in store.' : 'Nothing is being researched: points are stored until you choose.')
    : 'The computer plays this country and chooses its research. Play it to choose yourself.';
  const byFolder = {};
  for (const o of r.options || []) (byFolder[o.folder_name || o.folder] ||= []).push(o);
  const days = cost => has(r.points_a_day) && r.points_a_day > 0 ? num(cost / r.points_a_day) : DASH;
  const rows = Object.entries(byFolder).map(([folder, list]) => list.map((o, i) => `<tr>
      <td>${i ? '' : esc(folder)}</td><td>${esc(o.name)}</td><td class="n">${o.year}</td><td class="n">${num(o.cost)}</td><td class="n">${days(o.cost)}</td>
      <td>${played ? btn(cur?.key === o.key ? 'Chosen' : 'Research', 'sm', `data-tech="${esc(o.key)}" ${cur?.key === o.key ? 'disabled' : ''}`) : ''}</td></tr>`).join('')).join('');
  return frameWin('Technology', `${now}${part('technology.note', `<p class="small" style="margin:.3rem .4rem">${note}</p>`)}
    ${part('technology.options', band('Open technologies') + (rows ? `<table class="tbl"><tr><th>Area</th><th>Technology</th><th>Year</th><th>Cost</th><th>Days</th><th></th></tr>${rows}</table>` : `<p class="small" style="margin:.3rem .4rem">None open now.</p>`))}`);
};
