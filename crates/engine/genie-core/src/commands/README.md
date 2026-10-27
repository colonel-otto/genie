# `commands`: the players' commands

A command is a player's input, as a recording stores it. The tick runs its commands first, before
any object updates. `World::run` (`run.rs`) dispatches each command to its handler: the
building commands, the unit commands and the players' each have their own dispatch there.

- Orders to units (INTERACT, MOVE, FOLLOW, GUARD, STOP, PATROL and the others) go on to
  `orders/`.
- `BUILD` places a foundation: the placement test and the payment (`placement.rs`), the new
  building (`new_building.rs`), its ground (`ground.rs`) and its builders (`builders.rs`).
- `BUILD_WALL` places a line of wall foundations (`wall.rs`).
- `QUEUE` and `RESEARCH` fill a building's production queue (`production.rs`,
  `building_commands.rs`).
- `CANCEL_BUILD` deletes an object (`delete.rs`).
- `GAME` runs a sub-command: diplomacy, the farm queues, treason and the others (`game.rs`).

`Command` (`mod.rs`) is the decoded command. A command that genie does not model yet is
`Command::Unmodelled`, and it stops its player.

## In the game

- `RGE_Command` and `TRIBE_Command` (`command.cpp`, `tcommand.cpp`), with `do_commands` and
  `do_command`'s cases.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`builders.rs`](builders.rs) | BUILD's builders: their task, the queue and the direct path, the wait for the foundation and the build command. |
| [`building_commands.rs`](building_commands.rs) | The commands of a building: SET GATHER POINT, the command queue and RESEARCH. |
| [`delete.rs`](delete.rs) | A player's deletion of one of its objects: `CANCEL_BUILD` (0x6a, `TRIBE_Command::do_command` case 0x6a), which the game's delete key sends. |
| [`game.rs`](game.rs) | GAME (0x67): `TRIBE_Command::do_command`'s case (`0x140e77b00`), which lets a player's sub-command for another player through only with the game's cheats, and its switch on the sub-command (`FUN_140e7c330`, `World::game`). |
| [`ground.rs`](ground.rs) | A new building's ground: the units on its foundation sent off, its terrain, the footprint clearing and its place. |
| [`mod.rs`](mod.rs) | The commands (`Command`) and their dispatch (`World::run`): the handlers of the players' orders, the buildings' placement and the production queue's. |
| [`new_building.rs`](new_building.rs) | A new building: BUILD's foundation and the building constructor, with its combat setup, its storage and its annexes. |
| [`placement.rs`](placement.rs) | BUILD's tests before its foundation: a farm on rice terrain, the pending debit, the builders, the placement test and the payment. |
| [`production.rs`](production.rs) | QUEUE: a building's production queue, its load, its units and technologies, and a new entry's costs. |
| [`run.rs`](run.rs) | A command's run in the command phase (`World::run`), a human player's order and the order units' reset. |
| [`wall.rs`](wall.rs) | A player's line of wall foundations: `BUILD_WALL` (0x69, `FUN_140e7b2e0`), which dragging a wall sends. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../combat/), [`construction`](../construction/), [`gathering`](../gathering/), [`groups`](../groups/), [`map`](../map/), [`math`](../math/), [`movement`](../movement/), [`objects`](../objects/), [`orders`](../orders/), [`players`](../players/), [`production`](../production/), [`technology`](../technology/), [`unit_ai`](../unit_ai/), [`vision`](../vision/), [`world`](../world/).
- Used by: [`construction`](../construction/), [`garrison`](../garrison/), [`objects`](../objects/), [`production`](../production/), [`world`](../world/).
<!-- /uses -->
