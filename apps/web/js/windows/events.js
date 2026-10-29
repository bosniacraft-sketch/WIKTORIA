// EVENTS AND DECISIONS. The popup shows what waits for a played seat: its events (one at a time, an
// option button each) and the computer's questions (peace offers, calls to arms). It reads EVENTS
// (GET /api/events) and DIPLO (GET /api/diplomacy), asked by js/live.js while something waits.
// The Politics window (js/windows/politics.js) lists the decisions open to the seat (from EVENTS too).
let popupHidden = false;   // the player put the popup away; it comes back when something new waits
let popupCount = 0;

function drawPopup() {
  const el = $('popup'), played = !!S?.country?.played;
  const events = played ? (EVENTS?.pending || []) : [], questions = played ? (DIPLO?.questions || []) : [];
  const count = events.length + questions.length;
  if (count > popupCount) popupHidden = false;
  popupCount = count;
  el.hidden = !count || popupHidden;
  if (el.hidden) { paint(el, ''); return; }
  let body;
  if (events.length) {
    const e = events[0];
    const opts = e.options.map(o => `<button class="btn opt" data-event-key="${e.key}" data-option="${o.index}"><span>${esc(o.name)}${o.index === e.ai_option ? ' <small>(the computer&rsquo;s pick)</small>' : ''}</span></button>`).join('');
    body = `<h3>${esc(e.title)}</h3><div class="pic">${e.province ? esc(e.province.name) : esc(S.country.name)}</div>
      ${e.desc ? `<p>${esc(e.desc)}</p>` : ''}${opts}
      <p class="small">Fired ${esc(prettyDate(e.fired))}. Not answered by ${esc(prettyDate(e.expires))}, the computer&rsquo;s pick is taken.</p>`;
  } else {
    body = `<h3>Diplomacy</h3>${questions.map(q => questionHtml(q, true)).join('')}`;
  }
  paint(el, `<div class="event paper frame">${body}<div class="acts"><span class="small">${count > 1 ? `${count} things wait for you` : ''}</span><button class="mini" data-popup-hide>Later</button></div></div>`);
}
