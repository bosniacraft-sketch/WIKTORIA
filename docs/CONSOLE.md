# Console (planned, not built)

One command system inside the running game, opened with the usual key (^ or ~). Two layers with the same
parser, help and autocomplete: plain commands for players (testing and debugging), and `dev_` commands for
modders and for finding out why something happens. Commands use the game's own routines (annexation, war,
peace, construction...), not shortcuts around them, so a cheat can't leave the game in a broken state.
A game in which a cheat was used is flagged in the save, so bug reports can tell. A text file of commands
can run at start (`-exec file.txt`) to set up test scenarios.

## Player commands
| Command | Does |
| --- | --- |
| `annex all` / `annex <TAG>` | annex every other country, or one, through the annexation routine |
| `annexpuppets all` / `annexpuppets <TAG>` | annex all your puppets (vassals and substates), or one |
| `invent <invention>` | gain one invention |
| `research all` / `research <tech>` | gain every technology, or one |
| `instaconstruct` | toggle: buildings, factories and units finish the day they are ordered |
| `instaresearch` | toggle: the technology being researched finishes at once |
| `homedefense <n>` | spawn n infantry regiments in your capital (no pops needed; marked as console units) |
| `money <n>`, `prestige <n>`, `infamy <n>`, `leadership <n>` | change the value |
| `militancy <n>`, `consciousness <n>` | set it for your pops (or the selected province) |
| `own <province>`, `control <province>`, `release <TAG>`, `vassal <TAG>` | change who owns what |
| `war <TAG> [cb]`, `peace <TAG>`, `cb <TAG> [type]` | declare war, force a white peace, get a casus belli |
| `relation <TAG> <n>`, `influence <TAG> <n>`, `sphere <TAG>` | diplomacy |
| `play <TAG>` | switch to another country |
| `event <id>`, `decision <name>` | fire an event or take a decision |
| `ai on/off` | the computer players |
| `date`, `speed <n>`, `pause`, `save <name>` | time and saving |
| `help [command]` | list commands or explain one |

## dev_ commands
- Why: `dev_why_pop <id>`, `dev_why_country <TAG>`, `dev_why_war <TAG>`, `dev_why_factory <state>`
- Inspect: `dev_province <id>`, `dev_country <TAG>`, `dev_market <good>`, `dev_modifiers <TAG|province>`, `dev_rank`
- Script: `dev_trigger <expr>`, `dev_effect <expr>`, `dev_event_test <id>`, `dev_script_coverage`
- Values: `dev_define <section.KEY>`, `dev_field <FLD_id>`, `dev_set <FLD_id> <value>` (live, no file edit)
- Simulation: `dev_tick <days>`, `dev_seed <n>`, `dev_profile`, `dev_checksum`
- Map: `dev_mapmode <name>` (supply limit, RGO size, factory workers, colonial points, AI fronts)
- Saves: `dev_dump <file>`, `dev_compare <original save>` (what differs from the original at the same date)

Several of these already exist as `wiktoria-cli` command-line tools (why, script-coverage, check-*, run). The
console calls the same code.


## First set, for the prototype
help, annex, annexpuppets, invent, research, instaconstruct, instaresearch, homedefense, money, war,
peace, play, event, dev_tick, dev_why_*, dev_trigger.
