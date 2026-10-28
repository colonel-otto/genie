# `world`: the world's state and its tick

`World` (`mod.rs`) holds the whole game state: the players with their objects, the map, the
pathing state, the random streams and the masters. This folder holds that state and the tick that
moves it on. The behaviours that a tick runs live in their systems. Many of them are methods of
`World` in their own folders.

- `setup.rs` builds the world from the save in a recording's header.
- `world_update.rs` is one tick: the command phase, the player loop and the steps after it. Read
  it first: it calls every system in the game's order.
- `object_step.rs` runs one object's update with its context.
- `after_step.rs` applies what the update collected, in the order of the game's update.
- `context.rs` holds the `Context`: what one object's update reads and writes of the world.
- `player.rs` holds a player: its objects, its attributes and its formations.
- `settings.rs` holds the game options that the simulation reads.
- `checkpoint.rs` computes the 11-word state block of each player. A recording checkpoints that
  block at a fixed interval of syncs, and genie compares its own block with it.
- `forced.rs` is a tool for development. It puts the game's own state after a tick into the
  world, so that the next tick starts from the game's state.

## In the game

- `BaseWorld::update` and `TRIBE_World` (`world.cpp`, `tworld.cpp`, `World.cpp`,
  `BaseWorld.cpp`).
- `WorldPlayerBase::update` (`WorldPlayerBase.cpp`): a player's objects in list order.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`after_step.rs`](after_step.rs) | An object's step and what follows it (`World::step_object`): what its Context gathered, applied to the world in the order the game's update makes them. |
| [`checkpoint.rs`](checkpoint.rs) | The per-player 11-word state block that recorded games checkpoint every N syncs. |
| [`context.rs`](context.rs) | What a tick's update of one object reads and writes (`Context`) and its walks. |
| [`forced.rs`](forced.rs) | The forced replay (dev only): the game's state after a tick (`GameState`) and the step that forces it on the world. |
| [`mod.rs`](mod.rs) | The world: its state and its tick. |
| [`object_step.rs`](object_step.rs) | One object's step with its context (`World::run_object`) and a unit AI run outside its update. |
| [`others.rs`](others.rs) | The world's objects as one object's update reaches them (FD-045-D19): `Others` to change the owner's other objects, `OthersRef` to read them, both by id. |
| [`player.rs`](player.rs) | A player (`Player`): its objects, attributes and formations, and the stop of its update. |
| [`settings.rs`](settings.rs) | The game options the simulation reads: the object `FUN_140c87b90` returns (`GameOptions`, 0x1088 bytes), which the header saves and its load `FUN_141305310` fills. |
| [`setup.rs`](setup.rs) | The world as the header save holds it (`World::new`): the next object id, the planners and the shape mesh the game's load builds. |
| [`tests.rs`](tests.rs) | Tests. |
| [`world_update.rs`](world_update.rs) | One world update (`BaseWorld::update`): the command phase, the moving objects and the players' relations, each player's frame and the update with its commands. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../combat/), [`commands`](../commands/), [`construction`](../construction/), [`groups`](../groups/), [`map`](../map/), [`math`](../math/), [`movement`](../movement/), [`objects`](../objects/), [`orders`](../orders/), [`ownership`](../ownership/), [`pathing`](../pathing/), [`players`](../players/), [`technology`](../technology/), [`unit_ai`](../unit_ai/), [`vision`](../vision/).
- Used by: [`combat`](../combat/), [`commands`](../commands/), [`construction`](../construction/), [`garrison`](../garrison/), [`gathering`](../gathering/), [`groups`](../groups/), [`movement`](../movement/), [`objects`](../objects/), [`orders`](../orders/), [`ownership`](../ownership/), [`players`](../players/), [`production`](../production/), [`technology`](../technology/), [`unit_ai`](../unit_ai/), [`vision`](../vision/).
<!-- /uses -->
