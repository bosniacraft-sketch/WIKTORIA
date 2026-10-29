# UI layout reference (D18)

What the original's windows hold and where, written from screenshots
(kept outside the repo). This describes layout and grouping only. The port draws its own art,
colours, fonts and icons: nothing below is to be copied pixel for pixel or redrawn to look like the original.

## Main screen
- **Top bar** across the top: one tab per country window, each with a small live summary under its name.
  - Budget: treasury and daily balance.
  - Technology: research in progress.
  - Politics: ruling party.
  - Population: total and growth.
  - Trade.
  - Diplomacy: at peace or at war.
  - Military: army and navy size and war exhaustion.
- **Top left:** the country's flag (with its rank), prestige, industry and military score, then the date and
  the speed controls under it.
- **Right edge:** the outliner, a collapsible list of armies, navies and influence targets.
- **Bottom right:** a minimap with map-mode buttons around it, and a menu button.
- **Bottom centre and left:** the message log (alliances made, wars and so on), showing a few lines at a time.
- **Bottom left:** the province window opens here, so the map stays visible.

## Province window (narrow, bottom left)
Name and state, a terrain picture, a life-rating bar and the owner's flag. One row each: RGO good and its
output, income, population, literacy, militancy. Three small pies: workforce, ideology and culture. Supply
limit and cores (flags). One row per building (fort, naval base, railroad), each with its level and an
"expand" button. Buttons to build an army or a navy. Colonies show the colonise and protectorate actions
instead.
A foreign province shows the owner's summary: rank, government, ruling party, score and rank lines, sphere,
overlord, allies, wars, and a "go to diplomacy" button.

## Country windows (large, centred, one per top-bar tab, each with its own tabs)
- **Production:** tabs for factories (per state, with factory slots, workers and profit), foreign investment,
  production (a goods grid of output per good, grouped as military, raw, industrial and consumer goods), and
  projects (each with its progress and the goods it still needs). Building a factory opens a picker: a list
  of factory types with cost and time, and on the right the selected type's inputs, workers needed and cost.
- **Budget:** left is daily income (a tax slider per strata with a pie, and gold), then the national bank, total
  funds, debt and loans list. Right is daily expenses (sliders for national stockpile, education,
  administration, social, military, with subtotals), then tariffs, the diplomatic balance and the projected
  daily balance.
- **Politics:** left is the government (ruling party, upper house pie, election date, party policies, voters'
  and peoples' ideology pies, important issues table). Right has tabs: reforms (social and political, each
  with its options), movements and rebels, decisions (a scrolling list with a picture, text and an enact
  button), release nations.
- **Trade:** left is the goods grid with price and trend per good. Right is the selected good (price history,
  supply and demand breakdown, automate and buy/sell controls, stockpile slider), with tables under it for
  government needs, factories, population, market activity, stockpile and common market.
- **Diplomacy:** tabs for great powers (8 cards with flags and scores), wars (each war with both sides and a war
  score bar), war justification and crisis. Under them is a country table with region filters (all,
  continents, neighbours, sphere, enemies, allies) showing per-country relations, influence per great power
  and opinion. Right is the selected country (rank, government, scores, population and cultures, cordial,
  friendly and sphere flags) and a grid of action buttons (form alliance, call ally, military access, war
  subsidies, declare war, command units, increase/decrease opinion, add to/remove from sphere, justify war
  and so on).
- **Military:** left is the leaders list (generals and admirals, with traits and assignments) and a stats block
  (war exhaustion, supply consumption, organisation regain, unit start experience, recruit time, combat
  width, dig-in cap, military tactics). Middle and right are the armies and navies lists (name, location,
  strength bar), build army and build navy (unit picker with cost and time), and units being built.
  Mobilise is under the army list, with the brigades it would raise and its production impact.
- **Ledger:** 8 paged tables (Previous/Next, page number in a round badge; sortable header row, striped rows,
  country rows start with a flag):
  1. Nation ranking: status, military, industrial, prestige and total score.
  2. National comparison: total population, provinces, factories, literacy, leadership, brigades, ships.
  3. Political systems: government, national value, ruling party, party ideology.
  4. Political reforms: one column per political reform.
  5. Social reforms: one column per social reform.
  6. Country population: one column per pop type (icon headers).
  7. Provinces (own country): population, average militancy, consciousness and literacy, main religion, culture,
     issue and ideology.
  8. Province population (own country): one column per pop type.

## Small elements
- **Tooltips:** a dark panel with a title, a separator and lines of text. The goods tooltip lists the top 5
  producers with flags.
- **Dialogs:** a title bar, flags on both sides for diplomatic offers, text, and two buttons.
- **Unit panel:** brigades with their strength and organisation bars, and split, merge, new unit and disband
  buttons with keyboard shortcuts.
- **Search:** a "find country or province" box opens from the minimap.

## Keyboard shortcuts (same keys as the original, so players' habits carry over)
Read from the `shortcut = "..."` line on each button in the install's interface/*.gui files (there is no
separate hotkeys file). The port should read them from the install at runtime where a matching window
exists, and use these defaults otherwise.
- **Country windows:** F1 production, F2 budget, F3 technology, F4 politics, F5 population, F6 trade,
  F7 diplomacy, F8 military. L is the ledger.
- **Map modes** (two keyboard rows): q, w, e, r, t, y, u, i, o, p for modes 1-10 and a, s, d, f, g, h, j, k, l
  for modes 11-19. The mode order follows the install's map-mode list.
- **Province window:** a builds an army, n builds a navy, f sets the national focus, s opens the pop screen,
  and x, c, v, b set the land and naval rally points and merge.
- **Unit panel:** s splits in half, r makes a new unit or reorganises, g merges.
- **Ledger:** LEFT and RIGHT page through it.
- **Everywhere:** ESCAPE closes a window or declines, ENTER agrees or confirms, and DEL erases a battle-plan
  drawing. The menu bar's toggle is `<`.
- The game's general keys (pause on space, speed on + and -, and so on) are not in these files. Take the
  common ones as defaults and let players rebind everything.
