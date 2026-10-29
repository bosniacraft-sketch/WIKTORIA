# Modding and data

Everything that defines the world is data. There are three layers you can change without touching Rust.

## 1. The game data folder (content)

Wiktoria reads content in the Victoria 2 text format from a **game data folder**. The folder is chosen,
in order, by `--install <folder>`, the `VIC2_DIR` environment variable, or the first line of
`vic2_path.txt` in the repository root. Today this is a Victoria 2 install; a bundled starter world in
the same layout is planned (see `docs/AI_STATUS.md`).

Where each file is looked for is itself data: the `FLD_*_FILE` and `FLD_*_DIR` entries of
`data/engine_rules.json`. The defaults are the usual layout:

| Rules entry | Default path | Holds |
| --- | --- | --- |
| `FLD_DEFAULT_MAP_FILE` | `map/default.map` | Names the map files: `definition.csv`, `provinces.bmp`, `region.txt`, `adjacencies.csv`, terrain, sea provinces. |
| `FLD_COUNTRIES_FILE` | `common/countries.txt` | Country tags and their files under `common/countries/`. |
| `FLD_CULTURES_FILE`, `FLD_RELIGION_FILE` | `common/cultures.txt`, `common/religion.txt` | Cultures and religions. |
| `FLD_POPTYPES_DIR` | `poptypes/` | One file per pop type (strata, needs, promotion). |
| `FLD_GOODS_FILE` | `common/goods.txt` | Trade goods and base prices. |
| `FLD_PRODUCTION_TYPES_FILE`, `FLD_BUILDINGS_FILE` | `common/production_types.txt`, `common/buildings.txt` | Factories, farms and mines, buildings. |
| `FLD_TECHNOLOGY_FILE`, `FLD_TECHNOLOGY_DIR`, `FLD_INVENTIONS_DIR` | `common/technology.txt`, `technologies/`, `inventions/` | Research. |
| `FLD_ISSUES_FILE`, `FLD_IDEOLOGIES_FILE`, `FLD_GOVERNMENTS_FILE` | `common/...` | Politics. |
| `FLD_UNITS_DIR`, `FLD_MIL_DAY_CB_TYPES_FILE` | `units/`, `common/cb_types.txt` | Units and casus belli. |
| `FLD_PROVINCE_HISTORY_DIR`, `FLD_COUNTRY_HISTORY_DIR`, `FLD_POP_HISTORY_DIR` | `history/...` | The start: owners, buildings, pops, country set-up. |
| `FLD_LOCALISATION_DIR` | `localisation/` | Display names (`key;English;...` CSV lines). |
| `FLD_DEFINES_FILE` | `common/defines.lua` | Engine defines (`key = value` per section). |

Events (`events/`) and decisions (`decisions/`) are run by the script engine in
`crates/wiktoria_simulation/src/script.rs` and `cond.rs`. `wiktoria-cli script-coverage --save <file>`
reports how much of a data folder's events and decisions the engine understands.

**Mods** layer on top of the base folder like Victoria 2 mods: `--mod <file.mod>` (repeatable). A later
layer replaces files with the same relative path, and `replace_path` hides a whole base folder. See
`crates/wiktoria_data/src/install_fs.rs`.

**Start saves.** The game is currently always built from a `.v2` save of the start date
(`start/1836.v2` by default, git-ignored). Building the start straight from `history/` is not
supported yet.

## 2. Engine rules (`data/engine_rules.json`)

About 570 numbers and switches the engine needs that are not in the content files: price steps, battle
and siege constants, AI weights, pop growth scale, setup switches, file locations. Each entry is

```json
{ "field_id": "FLD_PRICE_STEP", "value": 0.01, "value_type": "number", "note": "price change per day" }
```

Change a value, then run `cargo test` (a few tests pin specific numbers and will tell you what they
expect). Adding a new constant: add an entry here, read it in the system's `from_engine_rules`, and never
write the number into Rust code.

## 3. The look

- **Game window:** colours, sizes and fonts in `theme.toml`, pictures in `apps/viewer/ui_art/`, and any
  file in `themes/<name>/` overrides the default (D18, D21). `wiktoria --ui-maker` opens a slot editor.
  The `original` theme in `apps/viewer/themes/` can point slots at art in your own Victoria 2 install
  (`install:` paths, read at runtime, never copied).
- **Browser interface:** see the table in `apps/web/README.md`. Colours are in `style/theme.css` only;
  icons are drawn in `js/icons.js` and can be replaced by `icons/<id>.svg`.
