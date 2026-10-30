# `players/ai_module`

A computer player has an AI module. The game does not run the module's own update in a
playback, but the simulation reads and writes its state: the world player's notification hooks
write it, and the hear logic (`FUN_14162e1b0`) reads it. This folder holds that state, by
player, as plain columns.

- `mod.rs` holds the state: the information module's path map, the explorer and its search
  points, the tactical module's slot times, and the table of modules by player.
- `explored.rs` holds the explored-tile hook. The visible map queues the tiles that a computer
  player explored. The world hands them to the player's module before anything reads it.

The save holds the path map, the map size and the slot times. The explorer and the search
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
| [`tests.rs`](tests.rs) | Tests. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`movement`](../../movement/), [`objects`](../../objects/), [`vision`](../../vision/), [`world`](../../world/).
<!-- /uses -->
