# Wiktoria in the browser: what the page asks the game

`wiktoria-cli serve --save <file.v2> [--port 8036]` runs the game on a background thread and serves the
interface in `apps/web/` to a browser on this computer (127.0.0.1 only). The page is drawn entirely
by its own code and style files; the game only answers the questions below, as JSON. A number the
game does not have yet is sent as `null`, never invented: the page then shows a dash.

Everything is our own naming. Content (names of countries, provinces, goods) comes from the player's
install at run time, as everywhere else.

## Files

`GET /` and any other path that is not `/api/...` or `/map.bmp`: the file of that name under
`apps/web/` (`/` is `index.html`). Only files inside that folder are served.

## The clock

- `GET /api/clock` -> `{"date":"1836.1.2","day":0,"paused":true,"speed":3,"days_per_second":0.0}`
- `POST /api/clock?paused=true|false` and `POST /api/clock?speed=1..5`: the game starts paused.
  Speeds are days per second of wall time: 1, 2, 5, 15, as fast as it can.

## The player's seat

- `GET /api/countries` -> `[{"tag":"ENG","name":"United Kingdom","rank":1,"great_power":true,"exists":true}, ...]`
  in rank order, countries that own land first.
- `POST /api/seat?tag=ENG` chooses which country the top bar shows. It does not change who plays
  it: a country nobody plays is the computer's.

## Playing a country (D20)

The game starts observed: every country is played by the computer, also the one the save names as
the player's. Each country has one flag, computer or person; the page plays at most one country at a
time (the simulation allows any number).

- `GET /api/play` -> `{"players":["ENG"]}`: the countries a person plays (empty while observing).
- `POST /api/play?tag=ENG` takes control of ENG and makes it the seat. Every other country the page
  played goes back to the computer at once, which plans for it again from the state as it is.
  -> `{"tag":"ENG","name":"United Kingdom","played":true,"players":["ENG"],"released":["FRA"]}`;
  400 for an unknown country or the rebels.
- `POST /api/observe` gives every played country back to the computer.
  -> `{"tag":"ENG","name":"United Kingdom","played":false,"players":[],"released":["ENG"]}`.
- `country.played` in `/api/screen` says whether the seat is played.

## Orders

Orders go to the seat's country and only while it is played; otherwise they answer 409
`{"error":"the seat is observed: ..."}`. An order is put in the game's queue, which is carried out
at the start of a day; the server has it carried out at once, between two days, which is the same
thing, so the answer says what happened and the next `/api/screen` shows it. A refused order answers
400 `{"error":"why"}`; a carried-out one `{"ok":true,"text":"what was done","value":0.35}` (`value`:
the slider's value as set, `null` for the other orders). GET on an order path answers 405.

- `POST /api/budget?slider=<name>&value=<v>`: names `poor`, `middle`, `rich` (taxes), `education`,
  `administration`, `social`, `military` (spending), `tariff`. Values 0..1, the tariff -1..1. A
  value outside the country's limits is moved to the nearest allowed one, as a slider stops at its
  end: the limits are those of the slider and of the ruling party (lowest and highest tax, highest
  military spending, lowest and highest tariff), sent as `budget.limits`:
  `{"poor":[0.1,0.9],...,"tariff":[-0.5,0.5]}`.
- `GET /api/research` -> what the seat may research:
  `{"tag":"ENG","played":true,"civilized":true,"current":{"key":"...","name":"...","progress":0.4} or null,`
  `"points_stored":12.0,"points_a_day":8.4,"options":[{"key":"...","name":"...","folder":"army_tech",`
  `"folder_name":"Army","year":1836,"cost":3200.0}, ...]}`: the technologies it does not have whose
  year has come and whose predecessor in the same area it has, in the files' order, with their cost
  to this country this year. A played country with nothing chosen researches nothing and stores
  its points; the computer never picks for it.
- `POST /api/research?tech=<key>`: research this next. Points already put into another technology
  go back to the stored points (our choice; nothing is lost by changing one's mind).
- `POST /api/army/move?army=<id>&to=<province>`: send an army (its `id` from `outliner.armies`)
  to a land province, over land borders that are not impassable (the computer's own path rules).
  Refused while the army is held in a land battle of 7 days or less (D17), while it is on a fleet,
  or when no land path exists; to its own province it stops.

## Events and decisions

An event with two or more options that fires for a played country (a country event of it, or a
province event of a province it owns) runs its immediate part at once and then waits for the
player's choice. The computer's own pick is rolled when it fires; it is taken when the player has
not answered within 30 days of game time (our choice; time stands while the game is paused) or
when the country goes back to the computer. An event with one option runs at once. The computer
takes no decision for a played country.

- `GET /api/events` -> the seat's waiting events and open decisions:
  `{"tag":"ENG","played":true,"pending":[{"key":7,"id":1100,"title":"...","desc":"..." or null,`
  `"province":{"id":300,"name":"London"} or null,"fired":"1836.2.1","expires":"1836.3.3",`
  `"options":[{"index":0,"name":"..."},...],"ai_option":1}],`
  `"decisions":[{"key":"form_x","title":"...","desc":"..." or null,"allowed":false}]}`.
  `key` names the waiting event in the order; `ai_option` is what the computer would take.
  `decisions` lists those whose `potential` holds (empty while observed); `allowed`: `allow`
  holds now. Texts are the install's localisation (`<name>_title`, `<name>_desc` for decisions).
- `POST /api/event?key=7&option=0`: pick an option (from 0). Its effects run at once.
- `POST /api/decision?name=form_x`: take a decision; refused when `allow` does not hold.
- `country.waiting` in `/api/screen` = `{"events":1,"questions":0}`: what waits for the seat.

## Diplomacy: casus belli, war and peace

The computer never declares war, justifies a casus belli, makes peace or joins a war for a played
country. Orders go through the computer's own war rules.

- `GET /api/diplomacy` -> `{"tag":"ENG","played":true,"infamy":4.5,"infamy_limit":25.0,`
  `"justifying":{"target":"FRA","cb":"conquest","name":"...","progress":0.4,"discovered":false} or null,`
  `"cbs":[{"key":"conquest","name":"...","infamy":5.0}],` (types that can be justified; `infamy`:
  what the goal costs unjustified, paid in part if the justification is discovered)
  `"countries":[{"tag":"FRA","name":"France","relation":-20.0,"allied":false,"truce":false,"at_war":false,`
  `"infamy":2.0,"held":[{"cb":"conquest","name":"...","until":"1837.6.1"}]}],` (every other country
  with land, by name; `held`: casus belli the seat holds against it)
  `"wars":[{"name":"ENG-FRA war of 1836","attacker":true,"leader":true,"attackers":["ENG"],"defenders":["FRA"],`
  `"score":12.0,"goals":[{"index":0,"cb":"conquest","name":"...","actor":"ENG","receiver":"FRA",`
  `"province":{"id":12,"name":"..."} or null,"ours":true}],"talk":["FRA"]}],` (`score` for the seat's
  side; `talk`: the enemies it may offer peace to, one of the two leading a side)
  `"questions":[{"key":3,"kind":"peace" or "call","from":"FRA","from_name":"France","war":"...",`
  `"goals":[{"cb":..,"name":..,"actor":..,"receiver":..}],"attackers":null or true,"made":"...","expires":"..."}]}`.
  A peace question with no goals is a white peace; a call asks the seat to join `war` beside `from`
  (on the attackers' side when `attackers`).
- `POST /api/justify?target=FRA&cb=conquest`: start justifying. One at a time, not during a truce
  or a war with the target. Progress and discovery (with its infamy) are the computer's rules.
- `POST /api/war/declare?target=FRA&cb=conquest`: declare war with a casus belli held against the
  target; refused without one, against an ally, or at war already. A truce may be broken, with its
  penalty. Both leaders call their allies; a played ally is asked.
- `POST /api/peace/offer?war=<name>&to=FRA&goals=0,2`: offer peace in a war enforcing those goals
  (`index` in `wars[].goals`); no `goals` is a white peace. The goals are either all demanded of
  `to` or all given to it. The computer answers at once (`400 {"error":"FRA refuses the peace"}`);
  a played enemy gets a question.
- `POST /api/answer?key=3&accept=true|false`: answer a question. Refusing a call ends the alliance
  and costs prestige; refusing a peace means the same enemy offers again in that war only after
  30 days. Unanswered within 30 days (or when the country goes back to the computer), the
  computer answers as it would for itself.

## One call for the whole screen

`GET /api/screen?province=<id or empty>` -> everything the fixed parts of the screen show, in one
answer, so the page can ask a few times a second:

```
{
 "clock": {...as above...},
 "country": {
   "tag":"ENG","name":"United Kingdom","played":false,"rank":1,"great_power":true,
   "prestige":105.0,"industry":148.0,"military":97.6,
   "money":15240.0,"balance":312.4,
   "research":{"name":"Idealism","progress":0.64,"points_a_day":8.4},
   "literacy":0.38,"population":31960000,"militancy":1.2,"consciousness":3.45,"infamy":4.5,
   "ruling_party":"Conservative Party","government":"HM's Government",
   "focus_used":2,"focus_max":3,"diplomatic_points":3.2,
   "regiments":57,"ships":41,"manpower":120000,"leadership":34,
   "produces":["grain","iron","wine"],"exports":["iron","wine"],"imports":["grain","fish"],
   "alerts":["factory_building","can_reform","at_war"],
   "waiting":{"events":1,"questions":0}
 },
 "budget": {
   "tax":{"poor":{"rate":0.55,"income":420.5},"middle":{...},"rich":{...}},"gold":4.0,"income_total":535.1,
   "bank":200.0,"funds":20620.0,"debt":0.0,"interest":0.0,
   "spending":{"education":{"rate":0.5,"cost":98.3},"administration":{...},"social":{...},"military":{...}},
   "stockpile":{"land":0.92,"naval":0.92,"projects":0.92,"cost":501.1},
   "admin_efficiency":0.272,"subsidies":0.0,"expenses_total":756.3,
   "tariff":{"rate":0.0,"income":0.0},"balance":-221.3,
   "limits":{"poor":[0.0,1.0],"middle":[0.0,1.0],"rich":[0.0,1.0],"education":[0.0,1.0],"administration":[0.0,1.0],"social":[0.0,1.0],"military":[0.0,1.0],"tariff":[-1.0,1.0]}
 },
 "province": null or {
   "id":300,"name":"London","state":"Home Counties","owner":"ENG","controller":"ENG",
   "terrain":"plains","life_rating":35,"population":342350,"growth":188,
   "goods":"grain","output":12.4,"income":31.0,
   "workforce":[["farmers",0.62],["artisans",0.18],...],"ideology":[...],"culture":[...],
   "fort":1,"naval_base":0,"railroad":2,"cores":["ENG"],"supply_limit":null,"admin_efficiency":0.62
 },
 "outliner": {
   "armies":[{"id":1201,"name":"1st Army","men":30000,"province":"London","province_id":300,"to":null}],
   "navies":[{"name":"Home Fleet","ships":14}],
   "wars":[{"name":"...","against":["FRA"]}],
   "factories_building":[{"kind":"steel_factory","state":"Home Counties","days":212}]
 },
 "log": [{"date":"1836.3.1","text":"..."}]   // newest first, at most 30: wars declared, peaces, events fired for the seat
}
```

Shares are 0..1. Money is in pounds as the player sees it (not the save's x1000).
An army's `to` is the province at the end of its path (`null` while it stands).

## The map

- `GET /map.bmp?mode=political&width=2048&layers=terrain,owners,rivers,borders,glow&tag=FRA` -> a 24-bit
  BMP of the whole world in that map mode, drawn by `wiktoria_render::layers` from the content's pictures and
  the current owners. Modes: the labels of `wiktoria_render::MapMode`. `layers` (default: all) picks what is
  drawn over the water: terrain tint, owners (the mode's colours), rivers, borders, and a glow outside
  the country `tag` (default: the seat).
- `GET /map/paper.bmp` -> the install's map paper (`map/terrain/map_overlay_tile.dds`) as grey around 128, for
  the page to lay over the map picture with an overlay blend; 404 when the install has none. With the
  install's colour pictures (`colormap.dds`, `colormap_water.dds`, `water.dds` in the same folder) the
  terrain layer takes land and sea colours from them and shades owners by the land's light and dark.
- `GET /api/map` -> `{"version":12,"modes":["political",...],"layers":["terrain",...],"width":5616,"height":2160}`: `version`
  goes up whenever an owner or controller changes, so the page knows when to fetch the picture again.
- `GET /api/pick?x=0.512&y=0.33` (position in the picture, 0..1) -> `{"province":300}` or `{"province":null}`.

## The start

- `GET /api/install` -> `{"path":"D:\\...\\Victoria 2","ok":true,"version":"3.04","mods":[]}`: what the
  game was started with (the launcher page shows it; choosing a folder in the browser is not possible,
  so the real folder dialog belongs to the desktop program later).

## Not provided yet

What `wiktoria-cli serve` answers with `null` (or differently from the examples above), because the
running state does not hold the number:

- `budget.tax.*.income`, `budget.gold`, `budget.tariff.income`, `budget.spending.*.cost`,
  `budget.stockpile.cost`, `budget.subsidies`, `budget.expenses_total`: the day's money report is
  per world, with only one balance per country. `budget.income_total` is yesterday's tax alone;
  `budget.balance` is that report's balance (taxes and duties less salaries, goods, subsidies) and
  `country.balance` is what the treasury really changed by over the last day. Both are `null`
  before the first day has run.
- `country.exports`: the market keeps no exported amounts. `country.imports` lists the goods of
  which more than half of what the country bought came from the world market; `country.produces`
  the six goods its farms, mines and factories make most of, by value.
- `country.manpower` is the number of people in the country's soldier pops.
- `province.growth`, `province.supply_limit`. `province.income` is the day's output at today's
  price. `province.admin_efficiency` is the owner's (one figure per country). A sea province has
  `"sea":true`, `null` numbers and empty lists.
- `country.research` keeps its three keys when nothing is researched (`name` and `progress` null).
- `country.alerts` uses: `factory_building`, `at_war`, `no_research`, `in_debt`, `occupied`,
  `rebels`, `over_infamy`, `mobilised`, `focus_free`. `can_reform` is not worked out.
- `log`: wars declared, peaces, great powers rising and falling for everyone; mobilisation,
  elections, reforms, technologies, factory projects and the seat's own diplomacy orders for the
  seat. Events fired are not in it (the event log counts events by id, not by country); a played
  seat's waiting events are in `/api/events`.
- `/api/diplomacy`: no war goals can be added during a war yet, and the casus belli types that need
  no justification are not offered (their conditions are not worked out). Refusing a good peace has
  no penalty yet.
- `/api/install`: `version` is always `null`.
- `rank` is `null` for a country that owns no land; `POST /api/seat` answers `{"tag","name"}`,
  and 400 for an unknown tag.
- While a day is being worked out, `/api/screen` for the same seat and province answers with the
  state as it stood at most a fifth of a second and one day earlier, instead of waiting.

## Population

`GET /api/population?state=&province=&type=&sort=size&desc=true&offset=0&limit=50` -> the seat's
people, for the Population window. All parameters are optional: `state` (a state's number from
`states[].id`) or `province` narrows the place, `type` is a comma-separated list of pop types to
show (empty: all), `sort` is one of `size`, `type`, `culture`, `religion`, `location`, `militancy`,
`consciousness`, `literacy`, `unemployment`, `cash`, `needs` (largest first unless `desc=false`),
and `offset`/`limit` (at most 200) pick one page of pops. An unknown `sort` or a bad number is 400.

    {"tag":"AAA","exists":true,"played":true,"population":3858000,"focus_used":2,"focus_max":3,
     "states":[{"id":1,"name":"State One","population":1300000,"focus":"Focus Two",
                "provinces":[{"id":102,"name":"Province 2","population":393000}, ...]}, ...],
     "types":[{"key":"farmers","name":"Farmers","strata":"poor","colour":"#6a8a2a","population":2390000,"shown":true}, ...],
     "filter":{"state":null,"province":null,"types":[]},
     "summary":{"population":3858000,"pops":63,"literacy":0.231,"militancy":1.84,"consciousness":2.37,
                "unemployment":0.031,"cash":51234.5,"types":[["Farmers",0.62],...],"culture":[...],
                "religion":[...],"ideology":[...],"issues":[...]},
     "pops":{"sort":"size","desc":true,"offset":0,"limit":25,"count":63,
             "rows":[{"id":5001,"type":"farmers","type_name":"Farmers","size":57400,"culture":"Culture B",
                      "religion":"Faith One","province":{"id":116,"name":"Province 16"},"state":6,
                      "militancy":0.53,"consciousness":1.36,"literacy":0.48,"unemployment":0.05,
                      "cash":25.24,"needs":{"life":1.0,"everyday":0.6,"luxury":0.1}}, ...]}}

- `states` are the seat's states, most people first, each with its provinces (most people first);
  provinces without a state come last as "Other provinces" with `id` null. `focus` names the
  national focus set on that state, or null.
- `types` lists every pop type of the install in file order, with how many people of that type
  live in the chosen place (whatever the type filter); `colour` is the type's colour in the install.
- `summary` covers the chosen place and types: people-weighted literacy (0..1), militancy and
  consciousness (0..10), the share out of work, the pops' cash in pounds, and the eight largest
  shares by type, culture, religion, ideology and issue (shares of the people; ideology and issue
  shares are weighted by the pops' own percentages).
- `unemployment` is what the port knows of employment: factory workers share the staffed places
  of their state's open factories, the people of the farm or mine share the places it offers,
  everyone else is never out of work (0).
- `focus_max` is the number of focuses the country had in the starting save (the computer's
  focus points), `null` when unknown. Setting a focus is not an order yet.
- An offset past the end answers the last page. A country without land answers
  `{"tag":"XXX","exists":false}`.
## Diplomacy window

`GET /api/diplomacy` (above) also carries the Diplomacy window's overview (apps/cli/src/serve/diplomacy.rs):

- `"seat_great_power":true,"sphere_of_seat":"ENG" or null,`
- `"powers":[{"tag","name","colour":"#rrggbb","rank":1,"prestige","industry","military","total","sphere":3,`
  `"relation","opinion","influence"}]`: the great powers best first. `colour` is the country's map
  colour (the page draws its flag tile from it); `sphere` the members of its sphere; `relation`,
  `opinion` (its opinion level of the seat, 0 hostile, 1 opposed, 2 neutral, 3 cordial, 4 friendly,
  5 in sphere) and `influence` (its influence in the seat) are `null` on the seat's own card.
- `"nations":[{"tag","name","colour","rank","status":"great"|"secondary"|"civilised"|"uncivilised",`
  `"continent","neighbour","sphere_of","overlord","government","population","cultures":["primary","accepted"...],`
  `"prestige","industry","military","total","opinion","influence":[..],"levels":[..],"access","guaranteed"}]`:
  every other country with land. `continent` is its capital's; `neighbour` shares a land border
  with the seat; `opinion` is the seat's opinion level of it (`null` unless the seat is a great
  power); `influence` and `levels` are each great power's influence and opinion level in it, in
  the order of `powers` (`null` for itself). Joined by `tag` with `countries` above.
- `"all_wars":[{"name","attackers":[..],"defenders":[..],"score":-12.0,"side":"attacker"|"defender"|null}]`:
  every running war, `score` from the attackers' side (-100..100), `side` the seat's.
- `"crisis":null` (the engine has no crisis yet), `"continents":[..]` (names, for the filters).

Scores are yesterday's (before the first day: those worked out at the start); a missing one makes
`total` `null`. Influence and opinion come from the diplomacy state; without it, influence and
sphere are the save's and opinions are `null`. The window's actions other than declare war,
justify and offer peace (alliances, access, subsidies, influence actions...) have no order yet.
## Politics

`GET /api/politics` -> the seat's government, parties, upper house, ideologies, issues, reforms,
movements, rebels and the countries it could release (`null` for an unknown seat). Read-only: the
engine has no player order for reforms or for releasing a country yet (`can_reform` and
`can_release` are `false`), so the page draws those buttons disabled. Decisions stay in
`GET /api/events` / `POST /api/decision` above.

```
{"tag":"ENG","played":true,"date":"1836.1.2",
 "government":{"key":"...","name":"...","elections":true} or null,
 "ruling_party":{"id":12,"name":"...","ideology":"conservative","ideology_name":"..."} or null,
 "national_value":"..." or null,"last_election":"1834.1.1" or null,"next_election":"1838.7.1" or null,
 "population":31960000.0,"militancy":1.2,"consciousness":3.4,
 "ideologies":[{"key":"conservative","name":"..."}, ...],          (file order)
 "upper_house":[["conservative",0.5], ...] or null,                 (every ideology, file order)
 "voters":[...] or null,"people":[...] or null,                     (same form)
 "parties":[{"id":12,"name":"...","ideology":"...","ideology_name":"...","ruling":true,"allowed":true,
   "policies":[{"group":"economic_policy","group_name":"...","option":"...","option_name":"..."}]}],
 "issues":[{"key":"...","name":"...","group":"...","group_name":"...","share":0.24}],   (at most 10)
 "reforms":{"house_political":0.31,"house_social":0.12,"support_needed":0.5,
   "last_reform":"..." or null,"next_possible":"..." or null,
   "groups":[{"key":"...","name":"...","political":true,"next_step_only":true,"current":"...",
     "options":[{"key":"...","name":"...","current":true,"open":false,"wanted":0.02}]}]},
 "movements":[{"kind":"issue" or "independence","target":"...","target_name":"...","support":412000.0,"radicalism":3.2}],
 "rebels":[{"id":11,"kind":"...","kind_name":"...","name":"...","creates":"TAG" or null,"organisation":0.42,"pops":14,"people":61000.0}],
 "rebel_armies":{"armies":1,"regiments":4},
 "release":[{"tag":"...","name":"...","provinces":5}],
 "can_reform":false,"can_release":false}
```

- `parties`: the country's parties that exist at the date; `allowed`: the government lets its
  ideology stand. The ruling party's `policies` are the "party policies" of the window.
- `next_election`: the day the next election ends (last election + the government's time between
  elections + the campaign); `null` when the government holds none.
- `upper_house` is the country's house as it stands; `voters` the ideologies of its pops outside
  colonies weighted by their strata's vote under its franchise; `people` the ideologies of all its
  pops by size. `null` when there is nobody to count.
- `issues`: the issue options its people ask for most (share of all its people), party issues and
  reforms alike. `wanted` of a reform option is the same share.
- `reforms.house_*`: the upper house's support for a political / social reform (the computer passes
  one when it is above `support_needed`); `next_possible`: the last reform + the least time between
  two. `open`: the option could be passed next (the step rule and its `allow` conditions).
- `movements` and `rebels` are as the save had them: pops do not yet join or leave movements and
  factions during play. `people` of a faction counts its member pops' sizes; `rebel_armies` the
  rebel armies standing on the country's provinces.
- `release`: countries that own no land and have cores on the country's provinces, with how many.
## Military

`GET /api/military` -> what the Military window shows for the seat (`null` for an unknown country):

```
{"tag":"ENG","name":"United Kingdom","played":true,
 "stats":{"war_exhaustion":3.4,"supply_consumption":1.0,"org_regain":0.06,"start_experience":null,
   "recruit_time":null,"combat_width":30.0,"dig_in_cap":1.0,"tactics":0.55,"leadership":34.0,"regiments":57,"ships":41},
 "mobilisation":{"mobilised":false,"progress":null,"size":0.03,"impact":0.5,"regiments":24,"production_loss":0.015},
 "leaders":[{"id":7,"name":"...","kind":"general" or "admiral","personality":{"key":"...","name":"..."} or null,
   "background":{...} or null,"traits":[["attack",1.0],["morale",0.05]],"prestige":0.12,
   "assigned":{"kind":"army" or "navy","id":12,"name":"1st Army"} or null}],
 "armies":[{"id":12,"name":"1st Army","province":{"id":300,"name":"London"},"men":29400,"regiments":10,
   "strength":0.98,"organisation":0.71,"doing":"moving","to":{"id":301,"name":"..."} or null,"leader":"..." or null,"dig_in":0.0}],
 "navies":[{"id":40,"name":"Home Fleet","province":{...},"ships":14,"strength":0.9,"organisation":0.8,
   "doing":"in_port","to":null,"leader":null,"carrying":0}],
 "units":[{"key":"infantry","name":"Infantry","land":true,"kind":"infantry","active":true,"days":120.0,
   "cost":132.5 or null,"goods":[["small_arms",10.0],...]}],
 "building":[{"name":"...","kind":"infantry","kind_name":"Infantry","land":true,"province":{...},"progress":0.4 or null,"ends":"1836.5.1" or null}],
 "orders":{"move_army":true,"build":false,"mobilise":false,"leaders":false}}
```

- `stats`: `war_exhaustion` and `leadership` are the country's; `supply_consumption` is
  1 + the country's modifiers + the war or peace modifier (as the military score reads it);
  `org_regain` is what technologies add to the daily organisation regain (their `morale`);
  `combat_width` the base width plus technologies and inventions (at least 2), `dig_in_cap` and
  `tactics` (base plus `military_tactics`) likewise. `start_experience` and `recruit_time` are
  always `null`: the engine does not work them out.
- `armies[].doing`: `embarked` (on a fleet), `battle`, `siege`, `retreating`, `moving`, `dug_in`,
  `standing`; `navies[].doing`: `battle`, `moving`, `at_sea`, `in_port`. `to` is the end of the
  path. `strength` and `organisation` are shares of what the regiments or ships can hold (the
  organisation ceiling as the day's regain uses it for armies; the unit type's for ships).
- `leaders[].traits`: the personality and background summed, statistics that are not zero.
- `mobilisation.regiments`: brigades mobilising today would raise (`null` while mobilised);
  `production_loss`: the share of production lost while mobilised; `progress` while raising.
- `units`: every unit type of the install; `active` is the unit file's own availability
  (technologies that unlock types are not followed yet); `cost` the goods at today's prices.
- `building`: regiments and ships being built (from the save; the engine starts none yet).
- `orders`: which of the window's orders the engine takes from a player. Only the army move
  (`POST /api/army/move`, while the seat is played); building, mobilising and leaders are not
  orders yet.
## Trade

`GET /api/trade?good=<key>&days=<n>` -> every good, and the good `good` in detail for the seat
(`days`: how many days of its price history, 2..1095, default 365):

```
{"tag":"ENG","played":false,"date":"1836.3.1","days_kept":60,
 "goods":[{"key":"grain","name":"Grain","group":"raw_material_goods","group_name":"...","color":[r,g,b],
   "price":2.2,"base":2.2,"yesterday":2.19,"month":2.0,"on_sale":true,"supply":81234.5,"demand":79000.1}, ...],
 "good": null or {
   "key":"grain","name":"...","group_name":"...","price":2.2,"base":2.2,"yesterday":2.19,"month":2.0,
   "history":{"from":"1836.1.1","prices":[2.2,2.2,...]},          // one a day, oldest first, the last is today
   "world":{"supply":81234.5,"demand":79000.1,"fill":0.97,"sold":1.0},
   "seat":{"made":{"farms":..,"factories":..,"artisans":..,"total":..},
           "bought":{"state":..,"factories":..,"pops":..,"total":..},"fill":0.95,"sold":1.0,"imported":0.1},
   "government":{"land":3.5,"naval":8.2,"construction":14.0,"bought":12.0},
   "stockpile":{"amount":55.4,"sliders":{"land":0.3,"naval":0.35,"projects":0.75}},
   "factories":[{"kind":"steel_factory","name":"...","state":"...","level":2,"makes":true,"made":210.2,"used":null,"closed":false,"building":false}],
   "population":{"bought":20.4,"artisans_made":38.2},
   "market":{"makers":[{"rank":1,"tag":"ENG","name":"...","units":2732.1,"share":0.15}],"buyers":[...]},
   "common":null or {"leader":"ENG","members":[{"tag":"ENG","name":"...","made":450.7,"bought":420.5}]},
   "orders":{"buy":false,"sell":false,"automate":false,"stockpile":false}
 }}
```

Goods are listed in the order of the install's goods file, with its groups (`group` is the file's
category key, `group_name` its name). Prices are in pounds; amounts are units of the good.

- `yesterday`, `month`: the price 1 and 30 days ago, from the server's own price book (the engine
  keeps today's price only). The book starts when the server starts and keeps one price a day for
  three years (`days_kept` days so far); before that the two are `null`.
- `supply`, `demand` (and `world`): the last day's supply and paid demand of the whole world,
  `null` before the first day has run. `fill` is the share of paid demand the market served,
  `sold` the share of supply that found a buyer (the market's smoothed shares, `State::market_fill`
  and `market_sold`).
- `seat.made`: the seat's units of the last day by its farms and mines, factories and artisans;
  `seat.bought`: its paid demand by the state (its stockpile), its factories' inputs, its pops'
  needs (with artisans' inputs). `fill`, `sold`, `imported`: its home-market shares. All `null`
  before the first day.
- `government`: what army and navy need a day at a full slider and the building work can take
  (`economy::CountryMoney`), and what the state bought yesterday. `stockpile.amount` is in store.
- `factories`: the seat's factories that make the good (`made`: units of the last day) or asked
  for it as an input (`used`).
- `market`: the eight countries that made and bought most of it in the last day, and the seat
  with its place when it is not among them. `share` is of the world's total.
- `common`: the sphere the seat leads or belongs to, leader first; `null` outside any sphere.
- `orders`: the engine has no buy, sell, automate or stockpile orders for a player yet, so all are
  `false` and the page draws those controls disabled. Needs by strata are not reported.
## Production

`GET /api/production`: the seat's industry, for the Production window (F1). Worked out at most
once a game day per seat (and again when the seat is taken or let go). Money is in pounds.

```json
{"tag":"AAA","name":"Country A","played":true,"can_build":false,"max_per_state":8,"treasury":2140.6,
 "policy":{"build":false,"expand":false,"reopen":false,"subsidise":false,"pops_build":true,"pops_expand":true,"pops_reopen":true},
 "states":[{"id":1,"name":"State One","provinces":6,"colony":false,"workforce":61200,"investment":184.5,
   "factories":[{"index":0,"kind":"steel_factory","name":"Steel factory","good":"steel","good_name":"Steel","level":3.0,
     "workers":28400,"places":30000,"output":6.12,"income":92.4,"inputs":51.2,"pay":30.1,"profit":11.1,"throttle":null,
     "status":"running","days_left":null,"subsidised":false,"loss_days":0}]}],
 "foreign_investment":{"here":[{"tag":"BBB","name":"Country B","amount":420.0}],"abroad":[]},
 "groups":[{"key":"military_goods","name":"Military goods"}],
 "goods":[{"key":"steel","name":"Steel","group":"industrial_goods","colour":"#5a6a7a","price":34.1,"rgo":0.0,"factories":6.12,"artisans":0.0,"total":6.12}],
 "projects":[{"state":"State One","kind":"machine_parts_factory","name":"Machine parts factory","what":"new","by":"state",
   "progress":0.61,"days_left":142,"price":null,"paid":null,"needs":[{"key":"cement","name":"Cement","amount":12.5}]}],
 "kinds":[{"key":"cement_factory","name":"Cement factory","good":"cement","good_name":"Cement","days":365,"cost":402.0,
   "goods":[{"key":"lumber","name":"Lumber","amount":60.0}],"inputs":[{"key":"coal","name":"Coal","amount":2.0}],
   "workforce":10000,"jobs":[{"type":"craftsmen","name":"Craftsmen","share":0.8}],"owner":{"type":"capitalists","name":"Capitalists"},"available":true}]}
```

- `states`: every state the country owns land in, with or without factories, in state order.
  `workforce`: people of the factory worker types living there; `investment`: what its capitalists
  have put aside (the fund their project is paid from).
- A factory's `status` is `running`, `closed`, `opening` (new, `days_left` until it opens) or
  `building` (a level being added). `workers`, `places` and `output` (units a day) are today's as
  the market will see them, `null` while it is shut or not open yet (`places` then comes from its
  type). `income`, `inputs`, `pay` are yesterday's sales, inputs bought and wages; `profit` =
  income - inputs - pay. `throttle` is always `null`: the engine has no such setting.
- `policy`: what the ruling party's economic policy lets the state and the capitalists do; `null`
  without a ruling party.
- `foreign_investment`: money others have in this country (`here`) and this country abroad, as the
  save has it (the engine does not move it).
- `goods`: every priced good of the four groups, with the country's output a day from farms and
  mines (`rgo`), factories and artisans. `colour` is the good's colour in the goods file.
- `projects`: building sites (`days_left`; `needs` = goods a state-paid site still lacks) and the
  capitalists' plans (`price`, `paid` from the state's fund; `progress` = paid / price).
- `kinds`: every factory type with its building goods (`goods`), `cost` at today's prices, `days`,
  its inputs a day per level, places per level (`workforce`) and their shares by pop type, and
  `available` (the country has the technology or invention for it).
- `can_build` is `false`: there is no player order to build, enlarge, reopen or subsidise a factory
  yet, so the window's Build button is drawn disabled.
