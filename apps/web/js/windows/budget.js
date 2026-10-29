// BUDGET. One window = one file. It reads S.budget (see API.md) and shows it. While the seat is
// played (S.country.played) the tax, spending and tariff sliders move: each sends its order
// (POST /api/budget, js/main.js) and the allowed part of the track is marked; observed, they only show.
// part(name, markup) gives a part our own name, written window.panel.row.
WIN.budget = S => {
  const b = S.budget || {}, tax = b.tax || {}, sp = b.spending || {}, st = b.stockpile || {};
  const played = !!S.country?.played, lim = b.limits || {};
  // A live slider of share 0..1; the tariff (-1..1) is shown on the same track, its middle at 0.
  const live = (name, tariff = false) => {
    if (!played) return null;
    const [lo, hi] = lim[name] || (tariff ? [-1, 1] : [0, 1]), share = v => tariff ? (v + 1) / 2 : v;
    return { name, lo: share(lo), hi: share(hi), title: `allowed ${pct(lo)} to ${pct(hi)}` };
  };
  const taxRow = (name, label, t = {}) => part(`budget.income.tax_${name}`, `<div class="row"><span class="rowhead">${label}</span><div class="rowbody">${pops(4)}${slider(t.rate, live(name))}<span>${pct(t.rate)}</span><span class="value">${money(t.income)}</span></div></div>`);
  const spendRow = (name, label, s = {}, extra = '') => part(`budget.expenses.${name}`, `<div class="row"><span class="rowhead">${label}</span><div class="rowbody">${pops(2)}${slider(s.rate, live(name))}<span>${pct(s.rate)}</span><span class="value">${money(s.cost)}</span>${extra}</div></div>`);
  const note = played ? '' : `<p class="small" style="margin:.2rem .4rem">The computer plays this country: the sliders show its budget. Play it to set them yourself.</p>`;
  return frameWin('Budget', `${note}<div class="cols">
  <div class="col">
    ${part('budget.income.header', band('Daily Income'))}
    ${taxRow('poor', 'Tax (Poor)', tax.poor)}${taxRow('middle', 'Tax (Middle)', tax.middle)}${taxRow('rich', 'Tax (Rich)', tax.rich)}
    ${part('budget.income.gold', `<div class="rowbody">${ic('gold')}<b style="flex:1">Gold</b><span class="value">${money(b.gold)}</span></div>`)}
    ${part('budget.income.total', `<div class="right"><span class="value big">${money(b.income_total)}</span></div>`)}
    <div class="kv">
      ${part('budget.funds.national_bank', `<div><span>National bank</span><span>${money(b.bank)}</span></div>`)}
      ${part('budget.funds.total', `<div><span>Total funds</span><span>${money(b.funds)}</span></div>`)}
      ${part('budget.debt.total', `<div><span>Total debt</span><span>${money(b.debt)}</span></div>`)}
      ${part('budget.debt.interest', `<div><span>Interest</span><span>${money(b.interest, 2)}</span></div>`)}
    </div>
  </div>
  <div class="col">
    ${part('budget.expenses.header', band('Daily Expenses'))}
    ${part('budget.expenses.subsidies', `<div class="rowbody">${ic('subsidies')}<b style="flex:1">Industrial Subsidies</b><span class="value">${money(b.subsidies)}</span></div>`)}
    ${part('budget.expenses.stockpile', `<div class="row"><span class="rowhead">National Stockpile</span>
      <div class="rowbody"><div class="sub">${ic('army')}${slider(st.land)}${ic('navy')}${slider(st.naval)}${ic('construction')}${slider(st.projects)}</div><span class="value">${money(st.cost)}</span></div></div>`)}
    ${spendRow('education', 'Education', sp.education)}
    ${spendRow('administration', 'Administration', sp.administration, `<span class="small">efficiency ${pct(b.admin_efficiency, 1)}</span>`)}
    ${spendRow('social', 'Social Spending', sp.social)}
    ${spendRow('military', 'Military Spending', sp.military)}
    ${part('budget.expenses.total', `<div class="right"><span class="value big">${money(b.expenses_total)}</span></div>`)}
    ${part('budget.tariffs', band('Tariffs') + `<div class="rowbody">${ic('tariffs')}${slider(has(b.tariff?.rate) ? (b.tariff.rate + 1) / 2 : null, live('tariff', true))}<span>${pct(b.tariff?.rate, 1)}</span><span class="value">${money(b.tariff?.income)}</span></div>`)}
  </div>
</div>
${part('budget.balance', `<div class="foot"><span>Projected Daily Balance</span><span class="value big ${has(b.balance) && b.balance < 0 ? 'bad' : ''}">${has(b.balance) ? (b.balance < 0 ? '&minus;' : '') + num(Math.abs(b.balance), 1) + ' &pound;' : DASH}</span></div>`)}`);
};
