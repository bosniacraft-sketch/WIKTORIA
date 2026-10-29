# Wiktoria

**Wiktoria** is a grand-strategy game engine written from scratch in Rust, in the spirit of
Victoria 2: pops, a world market, factories, politics, diplomacy, colonies and war, simulated day by day.
It reads content in the Victoria 2 text format, so the same kinds of files (countries, provinces, pops,
goods, technologies, events, decisions) drive it.

> ## Handover notice
>
> **The original author is no longer working on Wiktoria.** The project is handed over to the community
> as it stands, under the MIT licence. There is no maintainer, no support and no roadmap beyond what is
> written in this repository. Fork it, rename it, finish it, or take parts of it for your own game: you
> are free to do all of that. If you pick it up, start with [`docs/AI_STATUS.md`](docs/AI_STATUS.md)
> (an honest status report, written so that a person or an AI assistant can continue the work) and
> [`CONTRIBUTING.md`](CONTRIBUTING.md).

Wiktoria is a fan project and a homage. It is not affiliated with or endorsed by Paradox Interactive.
This repository contains no Paradox code, art, maps, text or data.

## What works today

- **Simulation core** (about 65,000 lines of Rust, 500+ tests): pops (growth, promotion, migration,
  literacy, militancy), farms and mines, artisans, factories, a world market with daily prices, state
  budgets and loans, elections and reforms, rebels and movements, diplomacy and spheres, crises,
  colonies, technology and inventions, events and decisions with a script engine.
- **War**: armies and navies, movement, supply and attrition, land and naval battles, sieges,
  occupation, war score and peace, fleets carrying armies, and a computer opponent that guards its
  borders in peace and gathers its armies in war.
- **Playing a country**: budget sliders, research, army orders, declaring war, making peace and choosing
  event options. Observe mode lets the computer play every country while you watch.
- **Two front ends**: a native game window (`apps/viewer`) and a browser interface served by the game
  itself (`apps/web`).
- **Test scenarios**: small set-ups that exercise one mechanic each and report PASS/FAIL.

## What does not work yet

- **It still needs game data in the Victoria 2 format and a start save.** Today that means a Victoria 2
  install (Heart of Darkness) that you own, plus a save of its 1836 start. A bundled, made-up starter
  world is the next planned step (see [`docs/AI_STATUS.md`](docs/AI_STATUS.md)).
- The native window has less interface than the browser version.
- Computer countries rarely end their wars by peace within a year, and several economy numbers are
  still off (listed in the status report).
- Saves cannot be written yet.

## Quick start (Windows)

1. Install Rust from <https://rustup.rs>.
2. Put the path of your Victoria 2 folder (the one with `v2game.exe`) in a file called `vic2_path.txt`
   next to this README, or set the `VIC2_DIR` environment variable.
3. Put a save of the 1836 start at `start\1836.v2` (made on the first day of a new game).
4. Run one of:
   - **`run.bat`**: the game window. The first build downloads its libraries and takes a few minutes.
   - **`wiktoria_web.bat`**: the game in your browser at <http://127.0.0.1:8036>.
   - **`play_scenario.bat siege_forts`**: a test scenario in the browser (run it without a name for the list).
   - **`cli.bat <command>`**: the command-line tools. The full list is at the top of `apps/cli/src/main.rs`.

On Linux or macOS use cargo directly:

```sh
cargo run --release --manifest-path apps/viewer/Cargo.toml          # game window
cargo run --release -p wiktoria-cli -- serve --save start/1836.v2   # browser game
cargo run --release -p wiktoria-cli -- scenario siege_forts         # headless scenario
```

## Building and testing

```sh
cargo build --release      # the engine and the command-line tool (no internet needed)
cargo test                 # all tests; they use small made-up worlds, no game files needed
cargo clippy
cargo build --release --manifest-path apps/viewer/Cargo.toml   # the game window (downloads crates)
```

The game window is a separate cargo workspace because it is the only part that uses crates from
crates.io (`eframe`, `image`, `rfd`). Everything else builds offline with the standard library only.

## Repository layout

| Path | What |
| --- | --- |
| `crates/wiktoria_data` | Loads game data: the script parser, map, provinces, countries, history, pops, localisation, defines, goods, modifiers, and Victoria 2 saves. No dependencies. |
| `crates/wiktoria_simulation` | The simulation: the whole game day. Depends only on `wiktoria_data`. |
| `crates/wiktoria_render` | Map camera, map modes, borders and picking into a plain pixel buffer. No dependencies. |
| `apps/cli` | `wiktoria-cli`: runs and serves the game, runs scenarios, and checkers that compare a run with saves. |
| `apps/web` | The browser interface that `wiktoria-cli serve` shows. Plain HTML, CSS and JavaScript, no build step. |
| `apps/viewer` | The native game window (`eframe`), its moddable art in `ui_art/` and its themes. |
| `data/engine_rules.json` | Every engine constant, by stable ID (`FLD_*`). Edit a value and re-run `cargo test`. |
| `scenarios/` | Test scenarios (see `docs/SCENARIOS.md`). |
| `docs/` | Architecture, modding, design decisions, the interface reference and the status report. |

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): how the code fits together and how one game day runs.
- [`docs/MODDING.md`](docs/MODDING.md): the data format, engine rules, themes and icons.
- [`docs/SCENARIOS.md`](docs/SCENARIOS.md): writing and running test scenarios.
- [`docs/DECISIONS.md`](docs/DECISIONS.md): the design decisions that code comments refer to (D2, D20...).
- [`docs/UI-LAYOUT.md`](docs/UI-LAYOUT.md) and [`apps/web/README.md`](apps/web/README.md): the interface.
- [`docs/CONSOLE.md`](docs/CONSOLE.md): the planned in-game console.
- [`docs/AI_STATUS.md`](docs/AI_STATUS.md): where the project stands and what is left, for people and AI assistants.
- [`CONTRIBUTING.md`](CONTRIBUTING.md): how to make changes.

## Licence

MIT, see [`LICENSE`](LICENSE). The bundled EB Garamond font is under the SIL Open Font License
(`apps/viewer/ui_art/fonts/OFL.txt`).
