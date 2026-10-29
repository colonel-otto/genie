# `objects/masters`: the definitions of the objects

A master is the definition of one kind of object for one player: its hit points, speed, armor,
attacks, costs, sprites and tasks. Each player has its own copy of every master. An effect
changes the copies of the players that it reaches.

The values are game data. They come from the player's masters in the save, and from the `.dat`
file for what the save does not hold.

- `mod.rs` holds the `Master` type and its tasks.
- `lookup.rs` finds a player's master by id, and the master and task that a work command takes.
- An effect changes a master through the attribute setters. The game has one setter per master
  type, and `dispatch.rs` picks it:
  - `static_setters.rs`: the static, animated, moving and action masters.
  - `combat_setters.rs`: the combat and missile masters (`RGE_`).
  - `tribe_setters.rs`: the combat and building masters of Age of Empires II's layer
    (`TRIBE_`).
- `derived.rs` updates the values that follow from a changed master.
- `attribute.rs` names the attributes that an effect changes.

## In the game

- The master classes (`MasterStaticObject.cpp` and the classes above it), with their attribute
  setters and their copy for an upgrade.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`action_type.rs`](action_type.rs) | The task action types (`Task::action_type`, `i16`) that more than one routine names: the action a task makes a unit do. |
| [`attribute.rs`](attribute.rs) | The attributes an effect changes on a master and on its objects (the game's attribute list: string 12200 plus the number names each). |
| [`class.rs`](class.rs) | The master classes (`+0x18`, an `i16`): the one definition of the class values. |
| [`combat_setters.rs`](combat_setters.rs) | The RGE combat and missile masters' setters: the armor and attack amounts. |
| [`derived.rs`](derived.rs) | The values the slice derives from a master after a change (`Master::follow`): the sprites' speed and facets, a moving unit's values. |
| [`dispatch.rs`](dispatch.rs) | The attribute setters by master type (slots 0x18, 0x20 and 0x28), the master copy (slot 8), the loader's defaults and the selected entries. |
| [`ids.rs`](ids.rs) | The master ids (`i16`, `crate::objects::masters::Master::id`): the one definition of the masters the engine names, by the `.dat`'s unit id. |
| [`lookup.rs`](lookup.rs) | A player's masters by id (`Masters`) and the work command's choice of master and task. |
| [`master_id.rs`](master_id.rs) | A master's id as one type: the master a command builds, trains or turns an object into, and a target's master. |
| [`master_type.rs`](master_type.rs) | The master types (`+0x8`, the object's kind at `+0xf`): the one definition of the type bytes. |
| [`mod.rs`](mod.rs) | What the engine reads of a player's masters: the tasks a unit can do and the masters a unit changes into to do them, and how an effect changes a master: the attribute setters (slots 0x18, 0x20 and 0x28 of every master type, `Master::set_attribute`), the master copy of an upgrade (slot 8, `Master::copy_from`) and the values the slice derives from a master (`Master::follow`). |
| [`static_setters.rs`](static_setters.rs) | The static, animated, moving and action masters' setters. |
| [`tests.rs`](tests.rs) | Tests. |
| [`tribe_setters.rs`](tribe_setters.rs) | The TRIBE combat and building masters' setters: the costs, the charge, the sprites and the train locations. |
| [`values.rs`](values.rs) | What the attribute setters share: the caps, the truncations, the sprite and class amounts, the damage sprites' and train locations' entries. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../../combat/), [`gathering`](../../gathering/), [`map`](../../map/), [`math`](../../math/), [`movement`](../../movement/), [`unit_ai`](../../unit_ai/).
<!-- /uses -->
