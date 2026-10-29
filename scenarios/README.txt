WIKTORIA TEST SCENARIOS
=======================

Each file here sets up one situation to try one part of the game: a siege, a battle, a landing
from the sea, an army starving, a war ending in peace, high taxes, factories, oil and rubber,
rebels, a computer country going to war, and a year of the computer playing alone.

Playing one
-----------
  play_scenario.bat siege_forts

builds the game, starts it with the scenario applied and opens it in your browser. The game starts
paused: read the first news lines (the scenario's name and what to look for), then press play.
In most scenarios you play the country named as the seat; "observed" scenarios give every country
to the computer and you watch. The scenario's checks are reported in the news as they fall due.

Run without a name, play_scenario.bat lists the scenarios.

Start saves
-----------
Scenarios start from a save of the original game. Put your own saves at
  start\1836.v2               (a save made on the first day of the 1836 start)
  start\1861_civil_war.v2     (a save made on the first day of the 1861 start)
in the Wiktoria folder. They are your files and are never added to the project.

Running one without the browser
-------------------------------
  cli.bat scenario siege_forts            runs it at full speed and prints PASS or FAIL per check
  cli.bat scenario siege_forts --days 30  a shorter run
  cli.bat scenario --list                 the list

A FAIL marked KNOWN is a known open bug the scenario is meant to show, not a new problem.

Writing your own
----------------
Copy a file and change it. The format is explained in docs\SCENARIOS.md.
