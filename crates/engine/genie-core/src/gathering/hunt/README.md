# `gathering/hunt`: the Hunt action

A villager sent to an animal or a tree with a gather task attacks it first (state 12, an Attack
sub-action). Once the target is dead, the villager walks to it (state 4), faces it (state 6) and
gathers (state 7). A full villager carries the load to a drop site (state 8). A villager that
hunts its own livestock can lead the animal toward the drop site first (state 10).

- `mod.rs` holds the action's state and its constants.
- `start.rs` starts the action, and holds its chase, its retarget and the walks to a point.
- `update.rs` is one update, with a target lost on the way and the attack on livestock.
- `gathering.rs`, `drop_off.rs` and `herding.rs` hold the states of the gathering, the drop-off
  and the herding.
- `sprites.rs` holds the hunt's sprites.
- `next_tree.rs` finds the next tree, the closest one to the hunt's drop site.
- `unit.rs` is the hunter's part of its update: its search, its walk after a drop and its
  drop-off.
- `unit_ai_side.rs` holds the hunt that a unit AI commands.

## In the game

- `TRIBE_Action_Hunt` (action type 110, `tact_hnt.cpp`).

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`drop_off.rs`](drop_off.rs) | State 8: the drop-off at the drop site; the target let go, the teardown and a case 6 without a target. |
| [`gathering.rs`](gathering.rs) | The walk to a dead target (state 4), the face (6) and the gathering (7). |
| [`herding.rs`](herding.rs) | Leading own livestock to its drop site (state 10), facing an animal and the decay check. |
| [`mod.rs`](mod.rs) | The Hunt action (`TRIBE_Action_Hunt`, type 110, vtable `0x142ed70f8`): a villager sent to a tree (or an animal) with a gather task attacks it first (state 12, an `Attack` sub-action); once it is dead the villager walks to it (state 4, a `MoveTo` sub-action), faces it (6) and gathers (7), then carries the load to a drop site (8). |
| [`next_tree.rs`](next_tree.rs) | The hunt's search for another target when its class is one the unit AI does not attack (trees): the closest tree from the hunt's drop site, which the hunt then takes. |
| [`sprites.rs`](sprites.rs) | The hunt's sprites: standing, walking and the task's; an asked drop-off and the kind of what a unit carries. |
| [`start.rs`](start.rs) | The Hunt's start, its chase and retarget, a move's target and the walk to a point (states 11 and 15). |
| [`tests.rs`](tests.rs) | Tests. |
| [`unit.rs`](unit.rs) | A hunter's search, its walk after a drop and its drop-off. |
| [`unit_ai_side.rs`](unit_ai_side.rs) | The hunt the unit AI commands while it runs (`crate::unit_ai::UnitSide::hunt_now` through its view, `UnitView`): `HuntObject`'s unit part at once and its plan. |
| [`update.rs`](update.rs) | One update (slot 6, `0x140ac7a00`), a target lost on the way and the attack on live livestock (state 12). |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../../combat/), [`map`](../../map/), [`math`](../../math/), [`movement`](../../movement/), [`objects`](../../objects/), [`orders`](../../orders/), [`unit_ai`](../../unit_ai/), [`world`](../../world/).
<!-- /uses -->
