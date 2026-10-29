# Wiktoria's interface (browser version)

The game's interface, built like a website: structure in one place, looks in another, numbers from
the running game. Start it with

    wiktoria-cli serve --save <a Victoria 2 save or the 1836 start>

and open http://127.0.0.1:8036 in a browser. There is no build step: edit a file, reload the page.

Everything in this folder is our own work. Names of countries, provinces and goods, and the map
picture, are sent by the running game from the player's own install; nothing of the original is
stored here.

## Where to change what

| I want to... | Edit |
| --- | --- |
| change a colour, the font, a texture | `style/theme.css` (the only file that names colours) |
| change how a piece is drawn (button, slider, number box, table, pie...) | `style/kit.css` |
| move or resize a fixed part of the screen | `style/screen.css` |
| change the window frame or the dialogs | `style/windows.css` |
| change the start screen | `style/launcher.css`, and the `#start` block in `index.html` |
| change what the top bar, country block, province window, outliner or log show | `js/hud.js` (one function per part) |
| change what is in a window | `js/windows/<window>.js` (`budget.js`, `technology.js`) |
| add a reusable piece | `js/kit.js` (a helper that returns markup) plus its look in `style/kit.css` |
| add or change an icon | `js/icons.js`; an icon that is not drawn yet shows as a brass token with two letters |
| replace an icon with my own picture | put `icons/<id>.svg` in this folder and add the id to `icons/index.json` (e.g. `["coal"]`); the file wins over the drawing. `/dev/icons.html` shows every icon with its id |
| change what the page asks the game, or how often | `js/live.js`, and `API.md` for what the game answers |
| clicks, keys, moving the map | `js/main.js` |

## How it fits together

- `js/live.js` asks the game for the whole screen twice a second and keeps the answer in `S`.
- Every draw function reads `S` and paints one part. A part is only repainted when it changed.
- A number the game does not have yet arrives as `null` and is shown as a dash, never invented.

## Adding a window

1. Create `js/windows/<name>.js` with `WIN.<name> = S => frameWin('Title', ...)`, built from the kit
   helpers (`band`, `slider`, `btn`, `pie`, `ic`, `money`, `pct`...). Copy `budget.js` as a start.
2. Wrap each part in `part('<name>.<panel>.<row>', ...)`: these are our own names.
3. Add a `<script>` line for it in `index.html`. A top-bar window is already in `TABS` in `js/hud.js`;
   a window without a file shows its frame and "not laid out yet".
4. If it needs numbers the game does not send yet, add them to `API.md` first, then to the server
   (`apps/cli/src/serve.rs`).

## What is real and what is not

- Real: the map, the clock, the chosen country's numbers, the budget figures, provinces, armies,
  wars, the log, as far as the simulation has them.
- Play or observe (D20): the start screen offers both, and "Play this country" / "Observe" next to
  the country's name switch at any time. While you play a country: the Budget window's tax,
  spending and tariff sliders send orders (the arrows move a step, a click on the track puts the
  knob there; the marked part is what the ruling party allows), the Technology window chooses the
  next technology, and an army picked in the outliner goes to the province you click next (Esc
  cancels). The computer plays every other country.
- Population (F5): the seat's states and provinces as a tree that filters the window, pies of the
  chosen pops by type, culture, religion, ideology and issue, one toggle per pop type, and the pops
  themselves in a table the game sorts and pages (size, culture, religion, place, militancy,
  consciousness, literacy, out of work, cash, needs met). The national focus count is real;
  setting a focus is drawn but not yet possible.
- Display only: the stockpile sliders and every other button inside windows.
- Diplomacy: great powers (cards with flag tiles drawn from the country colour, scores), every war
  with its war-score bar, war justification and crisis (a dash: the engine has none) as tabs; the
  country table (region filters, sortable: relation, opinion, each great power's influence) and the
  selected country. Live while played: declare war, justify, offer peace, answers; the other
  actions are drawn disabled ("not yet").
- Politics (F4): the government, ruling party, next election, party policies, the upper house,
  voters' and people's ideology pies and the important issues are real; the tabs show every
  reform with its option in force, those that may be passed next and how many ask for each, the
  movements and rebel factions (as the save had them), the decisions (Enact sends the order while
  you play) and the countries that could be released. Changing a reform and releasing a country
  are drawn but disabled ("not yet").
- Display only: the stockpile sliders and every other button inside windows.
- Military window (F8): the seat's leaders (generals and admirals, traits, assignment), armies and
  navies (where, strength, organisation, what they do), units being built, the unit types with cost
  and time, mobilisation and the military numbers; unit start experience and recruit time are dashes.
  While played, a click on an army picks it for the move order and closes the window. Build,
  mobilise and recruit leader are drawn but disabled ("not yet").
- Display only: the stockpile sliders and every other button inside windows.
- Trade (F6): the goods grid (price and trend against yesterday and a month ago), the chosen good's
  price history (kept by the server from the day it started), world supply and demand, what the
  seat made and bought of it, and tables for government needs, factories, population, market
  activity, stockpile and common market: all live. Buy, sell, automate and the stockpile slider
  are drawn but disabled ("not yet": the game has no trade orders for a player); needs by strata
  are a dash.
- Production (F1, `/api/production`): read only. Factories per state with their slots (level,
  workers, profit, status; throttle is a dash, the engine has none), foreign investment as the save
  has it, the goods grid of what the country makes a day by group, projects with progress and the
  goods still needed, and a Build factory picker (types with cost and time; the picked type's
  inputs, workers and building goods) whose Build button is disabled: no build order yet.
- Icons: the top-bar set and a few goods are settled drawings; every good, pop type, alert, unit kind and
  building also has a quick drawing marked `placeholder: true` in `js/icons.js`, meant to be replaced
  (edit the entry, or drop `icons/<id>.svg` in as above). Pop types show their icon in the Population window.
