# Test scenarios

A scenario is a small, ready-to-load setup that exercises one mechanic. The same file is played in the
browser (`play_scenario.bat <name>`, `wiktoria-cli serve --scenario <file>`) and run headless with a verdict
(`wiktoria-cli scenario <file>`). Files live in `scenarios/` at the repo root; the code is
`crates/wiktoria_simulation/src/scenario.rs` (format, setup, checks) and `apps/cli/src/scenario.rs` (commands).

The setup only edits the state before day 1 and the checks only read it: a scenario never changes how the
simulation runs.

## Commands

| Command | What |
| --- | --- |
| `wiktoria-cli scenario <file> [--days N] [--strict] [--save <file.v2>]` | run headless, print the setup, a line a month and one PASS / FAIL / KNOWN line per check. Exit code 1 when a check FAILs. |
| `wiktoria-cli scenario --list` | the scenarios in `scenarios/` with their seat, length and known bugs |
| `wiktoria-cli serve --scenario <file> [--save <file.v2>] [--port 8036]` | the browser game with the setup applied, the seat played (or every country observed), the speed set, and the checks reported as news lines when they fall due |
| `play_scenario.bat <name>` | Windows: build, then serve the scenario and open the page |

`<file>` is a path, or a name in `scenarios/` with or without `.txt`. `--save` overrides the file's base
save. `--days` overrides its length.

A check may name a **known bug**: when it fails it prints `KNOWN` instead of `FAIL` and does not set the
exit code, because the failure is expected until that bug is fixed. `--strict` counts KNOWN as FAIL. A
KNOWN check that starts to PASS means the bug is gone and the marker can be removed.

## File format

The install's script syntax (`wiktoria_data::script`): `key = value`, `key = { ... }`, `#` comments, quoted
text. Top-level entries:

| Entry | Meaning |
| --- | --- |
| `name = "..."` | required |
| `description = "..."` | one or two sentences on the situation |
| `what_to_look_for = "..."` | for the player: what to watch on screen. Shown as a news line when served. |
| `base_save = "start/1836.v2"` | the save the game is built from, looked up as given, next to the scenario file, in the game folder (WIKTORIA_ROOT, else the source tree) and in the install. Put start saves at `start/1836.v2` and `start/1861_civil_war.v2` (git-ignored, never committed). `history` (the install's own start without a save) is refused for now: the game is always built from a save. |
| `seat = TAG` | the country the player plays (no AI for it: its sliders stay, its armies only move on orders) |
| `observe = yes` | nobody plays: every country is the computer's and the page starts in Observe. Not with `seat` or `play`. |
| `days = N` | headless length (default 90) |
| `speed = 1..5` | starting speed of the browser game (default 3) |
| `known_bug = "..."` | a known open bug the scenario exposes as a whole (may repeat) |
| `setup = { ... }` | applied in order after the save is loaded, before day 1 |
| `check = { ... }` | repeated, one assertion each |

### Places

A province is a number (`300`) or the capital of a country (`capital:BEL`). Capitals keep a file right
without a province list at hand.

### Setup

Inside `setup`, an entry keyed by a country tag or a province runs the install's **effect language** in
that scope through the same runner as events (`script::run_effects`): anything an event option can do
(`treasury`, `prestige`, `add_country_modifier`, `relation`, `war_exhaustion`, `badboy`, `add_core`,
`secede_province`, `militancy` / `consciousness` / `money` inside `any_pop`, scopes such as `any_owned`).
An effect the runner does not know is skipped and printed as a warning.

```
ENG = { treasury = 50000 prestige = 10 }
capital:MEX = { any_pop = { militancy = 5 } }
MEX = { any_owned = { any_pop = { militancy = 9 } } }
```

Scenario-only commands, for what the effect language lacks:

| Command | Effect |
| --- | --- |
| `spawn_army = { owner = TAG at = <place> regiments = N unit = infantry name = "..." }` | an army of N full regiments of a land unit type. No pop stands behind them: losses come off the regiments only, and they are not reinforced. |
| `spawn_navy = { owner = TAG at = <place> ships = N unit = clipper_transport name = "..." }` | a fleet of N ships, in port (a land province) or in a sea zone (by number) |
| `move_army = { army = "<name>" to = <place> }` | orders a spawned army there, as the player's order does, over sea when it needs to (a fleet of the owner with room must exist) |
| `war = { attacker = TAG defender = TAG goal = <casus belli> }` | a running war with that goal. More `attacker` / `defender` lines add sides; no allies are called. A goal the install does not have gives a warning listing those it has. |
| `end_wars = TAG` or `end_wars = all` | ends the running wars of the tag (or every war), gives occupied provinces between their sides back to their owners and drops their battles and sieges |
| `clear_armies = TAG` | removes every army of the tag (and the battles they were in) |
| `controller = { province = <place> to = TAG }` or `controller = { of = TAG to = TAG }` | occupation of one province, or of every province the first tag owns |
| `owner = { province = <place> to = TAG }` | the province changes owner (and controller) |
| `fort = { province = <place> level = N }` | fort level, kept inside the install's maximum |
| `pop_money = { tag = TAG province = <place> pop_type = capitalists money = X }` | sets the money of every matching pop to X money units (tag or province required, pop type optional) |
| `set_money = { tag = TAG money = X }` | sets the treasury (the `treasury` effect adds) |
| `budget = { tag = TAG poor = 1 middle = 1 rich = 1 education = 0.5 administration = 0.5 social = 0 military = 1 tariff = 0.2 }` | budget sliders, kept inside the ruling party's limits as a player's are |
| `casus_belli = { actor = TAG target = TAG type = <casus belli> }` | the actor holds that casus belli for the install's valid time |
| `play = TAG` | another played country (the seat is played anyway) |

After the setup, owners and controllers changed by effects are copied to the military's provinces, and
the seat is played.

### Checks

```
check = { name = "Brussels falls" what = controller province = capital:BEL is = FRA day = 90 }
```

- `what`: the quantity (table below), with the arguments it needs: `tag`, `province`, `pop_type`, `good`,
  `a`, `b`.
- One expectation: `is = <value>`, `not = <value>` (text, a number, `yes`/`no`, or a place for a
  province-valued quantity), `min` and/or `max` (inclusive), or `change_min` and/or `change_max` (the
  value minus its value right after the setup).
- When: `day = N` (after N days), `date = 1836.6.1` (the first day at or after it), or nothing (at the
  end). A dated check the run does not reach is evaluated at the end.
- `known_bug = "..."`: the open bug a FAIL here is expected from (see Commands).
- `name`: the label printed; defaults to `what`.

Each check prints `PASS`, `FAIL` or `KNOWN`, the label, the date, the value measured (for a change also
where it started) and what was wanted.

| Quantity | Arguments | Value |
| --- | --- | --- |
| `controller`, `owner` | province | tag |
| `war` | a, b | yes when a running war has them on opposite sides |
| `at_war` | tag | yes / no |
| `wars` | tag optional | running wars (of the tag) |
| `wars_started` | tag | wars the tag declared during the run (the setup's wars and the save's not counted) |
| `regiments`, `men`, `armies` | tag, province optional | its regiments / men (strength x men per strength) / armies, standing in the province |
| `ships` | tag | ships |
| `sieges`, `battles` | province optional | sieges / land battles going on |
| `rebels` | tag or province optional | rebel regiments (in the tag's land) |
| `occupied`, `provinces`, `colonies` | tag | owned provinces controlled by others / owned / colonial |
| `money`, `prestige`, `war_exhaustion`, `badboy` | tag | |
| `factories`, `factory_levels` | tag | open factories (level 1+) / sum of levels |
| `population`, `pop_money`, `militancy`, `consciousness` | tag or province, pop_type optional | people / money in money units / size-weighted averages |
| `price` | good | today's price |
| `supply`, `demand` | good | world supply / paid demand on the last market day |
| `rgo_output` | tag or province, good optional | farm and mine output a day |
| `armies_moved` | | armies of countries now at war standing elsewhere than at the start |
| `lowest_gp_money` | | lowest treasury among the great powers |
| `nan_countries` | | countries whose money, prestige or war exhaustion is not a number |
| `battles_started`, `battles_finished`, `sieges_started`, `controller_changes`, `men_lost`, `risings`, `rebel_regiments_raised`, `peaces`, `wars_declared`, `wars_changed` (declared + peaces), `buildings_done` | | totals over the run |

## The shipped scenarios

| File | Mechanic | Expected | Known bug it exposes |
| --- | --- | --- | --- |
| `siege_forts` | sieges, fort levels | 15 regiments take Brussels (fort 1); 2 regiments do not take Bern (fort 6) in 120 days | |
| `field_battle` | land battle | 15 Prussian regiments beat 5 Austrian at Vienna, losing far fewer men | |
| `naval_invasion` | embarking, sailing, landing | a player-ordered army crosses from London to Amsterdam and besieges it | transport room is a stand-in |
| `ai_landing` | the army AI's landings | Britain, at war with an empty Netherlands, ships an army over and besieges a Dutch province | landings are a stand-in (`plan_landings`); transport room is a stand-in |
| `supply_attrition` | supply limit, attrition | 80 regiments in one province lose men every month | stack size vs the original not measured |
| `exhaustion_peace` | war exhaustion, peace | Anhalt, fully occupied, gains war exhaustion; peace within the year | failed-goal / refused-peace militancy not wired |
| `budget_taxes` | player budget | max taxes raise the treasury and slowly militancy | money flows |
| `factory_building` | capitalist and state building | Britain and Russia both open factories within a year | RUS builds nothing (#1); capitalists broke (#2) |
| `oil_rubber` | RGO hiring under small demand (1861) | oil and rubber produced, rubber price steady | oil/rubber output 0 (#4); prices decay (#5) |
| `rebels` | unrest, factions, risings | Mexico at militancy 10 forms factions that rise within a year | none |
| `ai_declares_war` | war AI with a casus belli | Russia declares on Krakow within 180 days | |
| `border_war` | our own war AI: posts, gathering, siege detachments | Prussia and Austria (both computer) fight, besiege and trade provinces | no peace within the year |
| `ai_watch` | the computer alone for a year (1861) | wars start or end, armies move, no great power deep in debt, no NaN | |

None of these has been run against the real content yet: there was no install in the session that wrote
them. The setup refuses what it cannot find (a tag, a capital, a unit type, a casus belli to hold), so a
first run shows any name that does not exist in the player's install; `war` only warns for an unknown
goal and the war runs without a ticking goal.
