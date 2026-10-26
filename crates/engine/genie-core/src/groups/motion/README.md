# `groups/motion`: a group's motion

A group walks as one unit. Its motion plans a path for the group's lead from the group's place,
over the same path levels that a unit's motion step keeps. Then it walks toward the waypoint of
the walked level, turns, and sets the velocity along its vectors.

- `step.rs` is the path step: its passes and the reached waypoint.
- `request.rs` asks the planners for a path level. If they find none, the motion gives up.
- `walk.rs` is the walk toward the waypoint and the motion's update.
- `levels.rs` holds the walked level's flags and the waypoints' points and lengths.

## In the game

- `BMotion` (`motion.cpp`): its update and its path step.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`levels.rs`](levels.rs) | The walked level's flags, its reset, a looping path's next waypoint and the waypoints' points and ground lengths. |
| [`mod.rs`](mod.rs) | A group's motion (FD-036): `BMotion`'s update (`FUN_14104a090`, motion.cpp): its path step (`FUN_14104a270`) over the path levels a unit's motion step keeps alike, planned for the group's lead from the group's place, the walk toward the walked level's waypoint with its turn (`FUN_141046270`, `FUN_14104c150`), then the velocity along the vectors. |
| [`request.rs`](request.rs) | The walked level's path request (state 2) and its end: the planners, the link of the found path, the give-up. |
| [`step.rs`](step.rs) | `BMotion`'s path step (`FUN_14104a270`): its passes and the reached waypoint. |
| [`tests.rs`](tests.rs) | Tests. |
| [`walk.rs`](walk.rs) | The walk toward the walked level's waypoint, and `BMotion`'s update with what it reads of the world. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`map`](../../map/), [`math`](../../math/), [`movement`](../../movement/), [`objects`](../../objects/), [`pathing`](../../pathing/), [`world`](../../world/).
<!-- /uses -->
