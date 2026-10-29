# Design decisions

These are the decisions the game was built on. Code comments refer to them by number (for example
"D20"). Numbers that are missing were internal process notes and are not part of the public project.
Change a decision by editing this file in the same commit as the code that depends on it.

## Goal

Wiktoria is an engine that runs grand-strategy content in the Victoria 2 file format, not an emulator of
another program (D22). It reads the same kind of text files (countries, provinces, pops, goods,
technologies, events), is 64-bit and multi-core, and has an interface of its own (D18, D21). No code from
Paradox, Project Alice, OpenV2 or OpenVic is used, and no Paradox files are in this repository.

## D2. Faithful, with fixes

| Kind | Examples | Policy |
| --- | --- | --- |
| Technical flaws | 32-bit memory limits, single-core speed, late-game slowdown, crashes | Always fixed |
| Clear bugs | siege rounding bug, economy edge cases nobody intended | Fixed by default, ideally behind a named switch |
| Design choices | anything intended, even if unpopular | Kept |

## D4. Determinism

- Same save + same inputs = same result, independent of thread count or machine.
- Random numbers come from explicit, seeded streams owned by the simulation, never from global state.
- The simulation is driven by an ordered list of player and AI commands, which keeps lockstep multiplayer
  possible later without a rewrite.

## D5. Saves

Victoria 2 `.v2` saves can be read and used as a starting point. Writing saves is not finished.

## D9. How close is close enough

- **Randomness uses our own seeded generator.** Reproducing another game's random generator is not a goal.
- **Watch for bias, not noise.** Differences that always lean the same way compound over years and get
  fixed even when small.
- **Long runs are judged by behaviour:** price bands, population, employment, industrial output, who
  industrialises.

## D10. The scoreboard

`wiktoria-cli score <reference> [<candidate>]` compares a run with a reference run on aggregate families
(prices, supply, population, pop money, employment, RGO income, tax base, treasury). `wiktoria-cli export`
produces the small CSV state tables it reads.

## D11. Engine relics are not replicated

Values that would overflow in a 32-bit engine simply keep growing here (64-bit numbers). Limits that are
part of how the game plays (militancy and consciousness stopping at 10) are kept.

## D13. Tweakable engine settings

Every engine constant lives in `data/engine_rules.json` under a stable ID (`FLD_*`) instead of in the
code, so it can be tuned and later offered to modders. Planned shape: a mod ships a small override file,
each setting has a default, a unit and an allowed range, and a bad value is refused with a message.

## D12 and D14. Population growth scale

The game does not model everything that removes people (war deaths beyond battles, famine detail), so
it keeps only a share of the people gained (`FLD_POP_GROWTH_SCALE`). The world should grow about 6% in 20
years, 16% in 40 and 30% in 64. Retune once the economy supplies needs properly.

## D15. The economy keeps the shape, not the exact numbers

The economy keeps the familiar structure (farms and mines, artisans, factories, pops buying needs, home
market first and the surplus to a world market, a state budget) and simplifies the machinery inside it. It
must hold three things: stable for 100 years, every lever pushes the right way, the data files are honoured.

## D16. Three tiers of closeness

- Tier A, what a player steers by (population, treasury direction, main prices, research pace, elections):
  within about 10%.
- Tier B, what a player notices (small pop types, militancy, migration, single prices): right direction,
  within about 25%.
- Tier C, dice and detail: plausible only.

Always: nothing runs away over 100 years, every lever pushes the right way, the data files are honoured.

## D17. Leaving a land battle: one week

An army may leave a land battle once it has lasted more than 7 days
(`FLD_MIL_MOVE_ORDER_BATTLE_MIN_DAYS`).

## D18. Our own interface

- Windows show the information a Victoria 2 player expects, where they expect it, but in our own look.
  No art, fonts or images from another game are copied or redrawn.
- Layout is relative and resolution-independent.
- Icons and images fit their slot; no code assumes an image's pixel size, so art can be swapped freely.
- Our art is moddable: PNGs in `apps/viewer/ui_art/` plus `theme.toml`; a `themes/<name>/` folder
  overrides any file it holds.

## D19. Name

The project is called **Wiktoria**. The crates are `wiktoria_data`, `wiktoria_simulation` and
`wiktoria_render`, the command-line tool is `wiktoria-cli` and the game window is `wiktoria`.

## D20. Observe mode and live country switching

- **Observe** is a first-class choice: every country run by the AI, any country can be inspected.
- **Switching country is live.** Each country is AI- or player-controlled through one flag. The country
  you leave gets its AI back at once; the AI stops for the country you take over.
- The simulation never assumes a player exists: zero, one or several player countries must all work.

## D21. The interface is drawn by code, styled by data

Frames, panels, buttons, tabs, bars, pies, tables and tooltips are drawn by code from a style. Colours,
fonts and sizes live in one style file; pictures are used only for content (flags, portraits, event art,
icons). Every screen is built from a small kit of reusable pieces.

## D22. An engine, not an emulator

The content's language (script commands, modifiers, defines) must mean what modders expect it to mean. How
it is computed, how fast, and how it looks are ours. We judge by whether the game plays sensibly, not by
whether a formula was copied.

## D23. We do not sterilise the game

Quirks and exploits players love come from the data files and stay. Flaws of the machine are fixed.

## D25. The ladder

Rungs, lowest first: one war you can watch; one country you can steer; an economy sane for ten years; a
world that behaves for thirty; mods load. Exactness comes last on every rung.

## D26. Rules first, presentation ours

Effort goes into the rules. Display features (fog of war, map textures) are designed for our own
interface when they are needed.

## D28. Standalone (2026-10-29)

The project is handed to the community as a standalone game. The goal is that it runs from a made-up
starter world bundled in `data/`, so no Victoria 2 install is needed. A Victoria 2 install stays an
optional data source. See `docs/AI_STATUS.md` for how far this has got.
