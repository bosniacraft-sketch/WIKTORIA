# Architecture

Wiktoria is a Rust workspace of three library crates and two programs, plus a browser interface.

```
                 game data folder (Victoria 2 text format)      data/engine_rules.json
                                     \                            /
                                      v                          v
  crates/wiktoria_data     load: script parser, map, provinces, countries, pops, history,
                           localisation, defines, goods, modifiers; read .v2 saves
                                      |
                                      v
  crates/wiktoria_simulation   the game day: economy, pops, politics, diplomacy, colonies,
                               war, AI, events and the script engine (deterministic)
                                      |
               +----------------------+----------------------+
               v                                             v
  apps/cli (wiktoria-cli)                         apps/viewer (wiktoria, the game window)
  run, serve the browser game (apps/web),         eframe window: map, menus, play/observe,
  scenarios, checkers, export, scoreboard         uses crates/wiktoria_render for the map
```

## Rules of the layering

- `wiktoria_data` and `wiktoria_render` have **no dependencies**. `wiktoria_simulation` depends only on
  `wiktoria_data`. Nothing in these three crates opens a window or draws on screen.
- `apps/viewer` is the only crate that pulls from crates.io, and it is a separate cargo workspace, so the
  core always builds offline.
- The simulation is **deterministic**: no clocks, no unseeded randomness. Random numbers come from
  explicit seeded streams (splitmix64) owned by the game state (D4).
- **No magic numbers in Rust.** Engine constants are read by stable ID from `data/engine_rules.json`
  (`FLD_*`) through `wiktoria_data::engine_rules::EngineRules`. Each system has a `...Rules` struct with a
  `from_engine_rules` constructor that reads its numbers once at start-up. Content (goods, units,
  technologies, events) comes from the game data folder.

## Loading (crates/wiktoria_data)

| Module | What it does |
| --- | --- |
| `script` | The parser for the Victoria 2 text format (`key = value`, `key = { ... }`, comments, quotes). |
| `install_fs` | Resolves relative paths over the base data folder plus mod folders, the way mods layer files. |
| `world` | `World::load`: reads everything into one `World` (definitions, map, provinces, countries, start pops). |
| `map`, `geography`, `bmp`, `dds` | `default.map`, `definition.csv`, the province picture, terrain, climates, continents, adjacency. |
| `defs`, `goods`, `production`, `modifiers`, `defines` | Cultures, religions, pop types, issues, ideologies, technologies, buildings, goods, production types, modifiers, `defines.lua`. |
| `countries`, `country_history`, `history`, `pops`, `localisation` | Countries, their start history, province history, pops, display text. |
| `setup` | The structural changes made to history pops before day 1 (each step switchable in the rules file). |
| `save`, `compare`, `popdiff` | Reading `.v2` saves and comparing them with the loaded world. |
| `config` | Finds `data/engine_rules.json` and the game data folder (`--install`, `VIC2_DIR`, `vic2_path.txt`). |

## The game day (crates/wiktoria_simulation)

`game::Game` owns the `World`, the script `State` (economy, pops, politics, events) and a `Core` with the
military, diplomacy, unrest, colony and building states. `Game::new(world, engine_rules, install, save)`
builds a game from a start save; `Game::day()` runs one day. Monthly steps run on the 1st, yearly ones on
1 January. The order of a day:

0. Orders of the countries a person plays (`player.rs`, D20).
1. Research, elections, reforms.
2. `Core::day`: army AI orders for countries at war, the military day (wars, war score, movement,
   battles, sieges), casualties and occupation applied to the state, mobilisation, army supply,
   construction, and on the 1st diplomacy drift, unrest, colonial points and AI railways; then war goals
   and peace offers on the foreign-minister cadence.
3. The market day (production, incomes, taxes, buying, prices), rank, events and decisions, pops' due
   steps (growth, literacy, mood, ideology, issues), national focus, industry (factories opening,
   closing, expanding), inventions, promotion, assimilation, migration.

Where a cross effect is not connected yet the code says `NOT WIRED:`; a simplified rule that stands in
for a missing one says `STAND-IN`; whole modules that are placeholders say `SCAFFOLDING`. Search for
these markers to find open work.

| Module | What it covers |
| --- | --- |
| `economy`, `market_day`, `market`, `production`, `consumption`, `trade`, `industry` | Farms and mines, artisans, factories, pop needs, prices, budgets, loans, the AI's factory building. |
| `pops`, `politics`, `reforms`, `unrest`, `game/factions`, `game/movements` | Pop growth and mobility, elections, reforms, militancy, rebels, movements. |
| `diplomacy/*` | Relations, alliances and treaties, influence and spheres, crises. |
| `colonies`, `buildings`, `research`, `rank` | Colonisation, forts, naval bases and railways, research, great-power rank. |
| `military/*` | Units, logistics and recruiting, movement, battles, sieges, war and peace, the army AI (`ai.rs`) and the war AI (`war_ai.rs`). |
| `script`, `cond`, `events` | The effect and trigger engine for events, decisions and scenario set-ups. |
| `player` | Countries played by a person and their orders. |
| `scenario` | The test-scenario format, set-up commands and checks. |
| `fixed`, `calendar`, `schedule`, `prof` | Fixed-point numbers, dates, the per-pop update schedule, profiling. |

## Front ends

- **`apps/cli`** (`wiktoria-cli`): `run` (headless), `serve` (the browser game, a hand-written HTTP/JSON
  server in `serve.rs` and `serve/*.rs`, contract in `apps/web/API.md`), `scenario`, `score`, `export` and
  the `check-*` tools that compare a run with a series of saves.
- **`apps/web`**: the browser interface. No build step; see `apps/web/README.md`.
- **`apps/viewer`** (`wiktoria`): the native window. `menu.rs` (start menu), `start.rs` (choose a start
  and a country), `ui.rs` (in-game interface), `mapview.rs` and `labels.rs` (the map), `sim.rs` (runs the
  game on a background thread), `theme.rs` and `slots.rs` (moddable look), `maker.rs` (a UI slot editor).

## Tests

Every crate has integration tests in `tests/`. They build tiny made-up worlds on disk or in memory, so
`cargo test` needs no game files. Scenarios (`scenarios/`, `docs/SCENARIOS.md`) are the higher-level
checks; they need a start save.
