# `groups/update`: a group's update

The world runs the update of each group before the objects of the group's player. This folder
holds that update.

- `entry.rs` runs a player's groups, deletes a group whose update ends it, and checks its members.
- `awake.rs` is the update of an awake group: its lead, the motion, the formation's turn and
  front, and a new layout.
- `moves.rs` runs the group's current command: a move, the members' moves and steps, the orders
  that are due and the idle timer.
- `checks.rs` holds what the update asks of the world: whether the units keep apart, whether a
  target is still there, whether a member is idle.

The idle timer draws from the world's random stream. That is the only draw of the world stream
outside the tick's own frame seeds.

## In the game

- `BUnitGroup::update` (`group.cpp`) and the group's destructor.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`awake.rs`](awake.rs) | An awake group's update: its lead, what follows the motion, the formation's turn and front, the lines' ranges and the relayout. |
| [`checks.rs`](checks.rs) | What the update asks of the world: the units kept apart, an interruption, rest, a target still to act on, the relics, a member's idleness, the sprites shown and the lead. |
| [`entry.rs`](entry.rs) | The update's entry: a player's groups, a group's update and its deletion, the members' check and a hibernating group. |
| [`mod.rs`](mod.rs) | A group's update (FD-036): `BUnitGroup::update` (`FUN_14103d910`), which the world runs for the groups a player owns before the player's objects, and the deletion of a group whose update returns 0 (the world list's unlink, then `BUnitGroup`'s destructor, `FUN_140fe1920`). |
| [`moves.rs`](moves.rs) | The current command's update: a move, the members' moves and steps, the order queue's due orders and the idle timer. |
| [`tests.rs`](tests.rs) | Tests. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../../combat/), [`math`](../../math/), [`movement`](../../movement/), [`objects`](../../objects/), [`orders`](../../orders/), [`unit_ai`](../../unit_ai/), [`world`](../../world/).
<!-- /uses -->
