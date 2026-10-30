# `players/ai_module`

A computer player has an AI module. The game does not run the module's own update in a
playback, but the simulation reads and writes its state: the world player's notification hooks
write it, and the hear logic (`FUN_14162e1b0`) reads it. This folder holds that state, by
player, as plain columns.

- `mod.rs` holds the state: the information module's path map, the explorer and its search
  points, the tactical module's slot times, and the table of modules by player.
- `search.rs` holds the explorer's search for an unexplored tile (`FUN_140b71990`) and its
  helpers: the path map's reads, the stack of points to try and the punt to a quadrant.
- `unit_ai_side.rs` holds the hear logic's explore case: the unit AI hands its explorer's
  notify to the owner, and a computer player's AI searches for the next tile. Its tasking and
  detasking of the explorer are not modelled.
- `explored.rs` holds the explored-tile hook. The visible map queues the tiles that a computer
  player explored. The world hands them to the player's module before anything reads it.

The save holds the path map, the map size, the quadrant scores and the slot times. The explorer and the search
points are not saved: the load makes them none and empty.

## In the game

- `TribeMainDecisionAIModule` (`player+0x7a8`), with the `TribeInformationAIModule` at
  `+0xbb28` and the `TribeTacticalAIModule` at `+0x25f88`.
- `WorldPlayer` slot 0x88 (`FUN_1416397b0`) and `FUN_140b8ab60`.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`explored.rs`](explored.rs) | The explored-tile hook (`WorldPlayer` slot 0x88, `FUN_1416397b0`): a computer player's map tells its AI module of a tile it explored for the first time. |
| [`mod.rs`](mod.rs) | A computer player's AI module (`player+0x7a8`, `TribeMainDecisionAIModule`, 0x28fb8 bytes). |
| [`search.rs`](search.rs) | The explorer's search for an unexplored tile (`FUN_140b71990`, "closestUnexploredTile") and its helpers on the information module: the path map's reads and the stack of points to try. |
| [`tests.rs`](tests.rs) | Tests. |
| [`unit_ai_side.rs`](unit_ai_side.rs) | A computer player's AI hears its explorer's notify (`WorldPlayer` slot 0x3c8, the hear logic `FUN_14162e1b0`, mType 0x202): the explorer's action waits for a point and no search of the unit AI gave one, so the AI picks the next tile ("I need to do a search for unit #%d"). |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`math`](../../math/), [`movement`](../../movement/), [`objects`](../../objects/), [`unit_ai`](../../unit_ai/), [`vision`](../../vision/), [`world`](../../world/).
<!-- /uses -->
