# Instructions for AI assistants working on this repository

1. Read `docs/AI_STATUS.md` first: it says where the project stands and what is open.
2. Then `docs/ARCHITECTURE.md` (layout and the game day) and `CONTRIBUTING.md` (the code rules).
3. Engine constants live in `data/engine_rules.json` under stable `FLD_*` IDs. Do not hard-code numbers
   in Rust.
4. Never add files, text, maps or art from a commercial game. Tests use small made-up worlds.
5. Keep `crates/wiktoria_data` and `crates/wiktoria_render` dependency-free and
   `crates/wiktoria_simulation` dependent only on `wiktoria_data`. crates.io dependencies belong only in
   `apps/viewer`.
6. `cargo test` and `cargo clippy` must pass before a change is done. Update `docs/AI_STATUS.md` when
   you finish or start a piece of open work.
