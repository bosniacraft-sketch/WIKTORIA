# Project status (for AI assistants and new maintainers)

Last updated: 2026-10-29, at the handover. Read this first, then `docs/ARCHITECTURE.md`,
`CONTRIBUTING.md` and `docs/DECISIONS.md`.

## One-paragraph summary

Wiktoria is a Victoria 2-style grand-strategy engine in Rust (~63,000 lines, 505 passing tests). The
simulation core is broad and runs a full game day (economy, pops, politics, diplomacy, colonies, war, AI,
events). It currently loads content from a Victoria 2 install in the original text format and builds the
game from a `.v2` start save, so it is **not yet standalone**. The original author has stepped away and
handed the project to the community under MIT; there is no maintainer.

## Ground truth

- Build: `cargo build --release`; tests: `cargo test` (offline, synthetic data only); lint: `cargo clippy`.
- Game window: `cargo run --release --manifest-path apps/viewer/Cargo.toml` (separate workspace).
- Browser game: `cargo run --release -p wiktoria-cli -- serve --save start/1836.v2`, then open
  `http://127.0.0.1:8036`.
- Headless: `wiktoria-cli run --save <file> --days N`, `wiktoria-cli scenario <name>`.
- Data folder: `--install`, `VIC2_DIR` or `vic2_path.txt`. Start save: `start/1836.v2` (git-ignored).
- Engine constants: `data/engine_rules.json` (`FLD_*`), read via `EngineRules` (`crates/wiktoria_data/src/engine_rules.rs`).
- Markers in code: `NOT WIRED:` (about 40), `STAND-IN`, `SCAFFOLDING`. Grep for them.

## Status by area

| Area | State | Main files |
| --- | --- | --- |
| Data loading, mods, saves (read) | Done and tested | `crates/wiktoria_data/src/*` |
| Map rendering | Done (political, provinces, population, culture, religion, states modes) | `crates/wiktoria_render`, `apps/viewer/src/mapview.rs` |
| Pops: growth, literacy, mood, promotion, migration, assimilation | Working; growth scaled by `FLD_POP_GROWTH_SCALE` (D12/D14) | `pops.rs` |
| Economy: farms/mines, artisans, factories, market, budgets, loans | Working, with known numeric problems (below) | `economy.rs`, `market_day.rs`, `industry.rs`, `production.rs` |
| Consumption order and trade | **SCAFFOLDING**: plausible guesses, constants hard-coded | `consumption.rs`, `trade.rs` |
| Politics: elections, reforms, upper house | Working | `politics.rs`, `reforms.rs` |
| Unrest: militancy, factions, movements, rebels | Working; rebel armies get no orders | `unrest.rs`, `game/factions.rs`, `game/movements.rs` |
| Diplomacy: relations, alliances, influence, spheres, crises | Working; AI alliances and access not wired | `diplomacy/*` |
| Colonies | Working; AI colonial steps not wired | `colonies.rs` |
| War: units, movement, supply, battles, sieges, occupation, war score, peace | Working | `military/*` |
| Army AI | Peace: capital reserve plus guard posts on threatened borders. War: small armies gather on the main army, big sieges leave a detachment | `military/ai.rs`, `game.rs` |
| War AI (declaring war, goals, peace) | Working, but **AI wars rarely end in peace within a year** | `military/war_ai.rs`, `military/war.rs` |
| Events, decisions, script engine | Most effects and triggers work; `script-coverage` reports the rest | `script.rs`, `cond.rs`, `events.rs` |
| Playing a country | Budget, research, army orders, war, peace, event choices; live switching and Observe mode | `player.rs`, `game/play.rs` |
| Browser interface | Most windows exist (budget, technology, politics, population, production, trade, diplomacy, military, events) | `apps/web`, `apps/cli/src/serve*` |
| Native window | Start menu, start/country picker, map, basic in-game UI; less than the browser | `apps/viewer/src/*` |
| Saving games | **Not built** (reading saves works) | none |
| In-game console | **Planned only** | `docs/CONSOLE.md` |
| Standalone starter world | **Not built** (next step, below) | none |

## Known problems (from the test scenarios)

Each is a `known_bug` in `scenarios/*.txt`, so the scenario prints KNOWN instead of FAIL:
- The computer does not make peace in a war within a year (`border_war`).
- Money collects in treasuries and with bureaucrats while capitalists and artisans go broke
  (`budget_taxes`, `factory_building`).
- Russia, Austria, Spain and the Ottomans build no factories (`factory_building`; the AI's stand-in
  wants free factory workers).
- Oil and rubber output is 0 (`rgo_staff_step` fires everyone under tiny paid demand), and the prices of
  rubber, oil, coffee and opium fall every year (`oil_rubber`).
- Overseas landing targets and the room per transport ship are stand-ins (`ai_landing`, `naval_invasion`).
- Militancy from failed war goals and refused peace offers is not wired (`exhaustion_peace`).

## Open work, in suggested order

1. **Standalone starter world** (decision D28). Goal: run with no Victoria 2 install.
   - Write a generator (a script or a `wiktoria-cli` command) that produces a small made-up world in the
     same folder layout under `data/starter_world/`: `map/default.map`, `definition.csv`,
     `provinces.bmp`, `region.txt`, `adjacencies.csv`, terrain, `common/*` (countries, cultures, religion,
     goods, production types, buildings, technology, issues, ideologies, governments, cb_types,
     defines.lua, static and event modifiers), `poptypes/`, `units/`, `technologies/`, `inventions/`,
     `history/` and `localisation/`. The synthetic worlds in `crates/wiktoria_render/tests/synthetic_world.rs`
     and `crates/wiktoria_simulation/tests/military_day.rs` show the minimum each loader accepts.
   - The game is built from a `.v2` save (`Game::new(world, rules, install, save)`). Either generate a
     start save for the starter world, or (better) add a path that builds the start state straight from
     `history/` (scenario `base_save = history` is reserved for this and currently refused in
     `apps/cli/src/scenario.rs`).
   - Point `config::install_dir` at `data/starter_world` when nothing else is configured, and rewrite the
     scenarios, which name Victoria 2 tags (ENG, PRU, MEX...), for the starter world.
   - Generated placeholder flags and map colours; the native window must not require install art.
2. **Peace in AI wars** (`military/war_ai.rs`): the AI should offer and accept peace once war score,
   exhaustion or occupation make the war pointless.
3. **Economy fixes** listed above; replace `consumption.rs` and `trade.rs` scaffolding.
4. **Move the interface into the native window** and retire the browser version (the original author
   wanted the game to run in a real game window, not a browser).
5. **Saving games**, then the console.
6. **AI that reacts to the world**: countries shifting ideology, alliances and rivalries in response to
   the player and to each other, not only on the battlefield.
7. Escorted landings and beachheads for the army AI.

## Conventions an assistant must keep

- No game files, text or art from a commercial game in the repository; tests use made-up data.
- No new numbers in Rust: add `FLD_*` entries to `data/engine_rules.json`.
- Keep the simulation deterministic and the core crates dependency-free.
- `cargo test` and `cargo clippy` must pass before a change is done.
