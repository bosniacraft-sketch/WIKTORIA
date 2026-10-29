# Contributing

Wiktoria has no active maintainer (see the handover notice in the README). These are the rules the code
was written under; following them keeps the codebase coherent for whoever comes next. If you fork the
project you are free to change them.

## Before you send a change

```sh
cargo test
cargo clippy
cargo build --release --manifest-path apps/viewer/Cargo.toml   # if you touched the game window or a shared crate
```

All three must pass. If you change a rule that a scenario checks, run it too:
`cargo run --release -p wiktoria-cli -- scenario <name> --save start/1836.v2`.

## Code rules

1. **Layering.** `wiktoria_data` and `wiktoria_render` have no dependencies; `wiktoria_simulation`
   depends only on `wiktoria_data`. No window or graphics code outside `apps/viewer` and `apps/web`.
   crates.io dependencies are allowed only in `apps/viewer`.
2. **No magic numbers.** Engine constants go in `data/engine_rules.json` under a stable `FLD_*` ID and
   are read in the system's `from_engine_rules`. Content (goods, units, events, technologies) comes from
   the game data folder, never from Rust code.
3. **Deterministic simulation.** No system clock, no unseeded randomness, no iteration over hash maps
   where the order changes the result. Use the seeded generators the game state owns.
4. **Mark what is not finished.** `NOT WIRED:` for a cross effect that is not connected, `STAND-IN` for
   a simplified rule standing in for a real one, `SCAFFOLDING` for a placeholder module. Remove the
   marker in the same change that finishes the work.
5. **Tests use made-up data.** Tests build small synthetic worlds; never commit files from a commercial
   game (data, maps, text, art or saves) to this repository.
6. **Clear names.** Prefer whole words (`regiments_per_transport`, not `rpt`) in new code.
7. **Decisions.** If a change contradicts `docs/DECISIONS.md`, update the decision in the same commit.
8. **Clean room.** Do not copy code from Paradox games, Project Alice, OpenV2 or OpenVic.

## Where to start

`docs/AI_STATUS.md` lists the open work by area, with the files involved. `git grep "NOT WIRED"` and
`git grep "STAND-IN"` list the smaller gaps in the code itself.
