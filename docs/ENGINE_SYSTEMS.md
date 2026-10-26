# The engine's systems

`genie-core` is genie's simulation core. It reimplements the simulation of Age of Empires II:
Definitive Edition, build 158041, bit for bit. It has no graphics, sound or input: other crates
add those. This page explains how the core is organised. It describes one tick of the world, one
object's update, the nineteen systems the code is sorted into, and how to find a behaviour.

Each folder of [`crates/engine/genie-core/src/`](../crates/engine/genie-core/src/) has a
`README.md`. It explains the system or the part in that folder and lists its files.

## Terms

| Term | Meaning |
| --- | --- |
| world | The whole game state: the players, their objects, the map, the pathing state, the random streams. The type is `World` (`world/`). |
| tick | One update of the world. A tick runs the players' commands, then every player's objects. The game calls it `BaseWorld::update`. |
| player | One side of the game. Player 0 is Gaia: the animals, trees, mines and relics. |
| object | Anything in the world: a villager, a soldier, a building, a tree, a sheep, a missile. |
| `Unit` | The type that holds one object, of any kind (`objects/unit/`). The name comes from the slice's first ports. |
| master | The definition of an object's kind for one player: its hit points, speed, tasks, costs. The `.dat` file gives the masters, and each player's copy changes with its technologies. |
| action | What an object does now: walk to a point (`MoveTo`), attack, gather, build, repair. An object holds a list of actions and runs the first one. |
| unit AI | The part of a unit that decides by itself: it picks targets, answers attacks and takes orders. |
| command | A player's input, as a recording stores it: an order to units, a building placement, a research. |
| order | What a command, or a unit AI, tells a unit to do. An order usually ends in a new action. |
| task | One entry of a master's task list: what the master can do to a kind of target (gather wood, build, repair). |
| port | The Rust reimplementation of one routine of the game. Its documentation names the routine. |
| `RGE_`, `TRIBE_` | The prefixes of the game's classes. `RGE_` classes are the Genie engine's base. `TRIBE_` classes are Age of Empires II's layer on top of it. |
| stop | A player whose update reaches a part that is not ported yet stops. Its objects freeze and the stop names the missing part (`GenieError::unsupported`). |

## One tick

`World::tick_with` (`world/world_update.rs`) runs one tick. The steps run in the game's order,
because every random draw and every list order shows in the result.

1. **The command phase** (`world/`). The tick count and the world time go on by `dt`. All random
   streams take the frame seed. Then the tick's commands run, one after the other
   (`World::run`, `commands/`).
2. **The player loop.** Gaia goes first, then players 1 to 8. For each player:
   1. The gates of the player open for its own units (`construction/gates.rs`).
   2. In an Empire Wars game, the first tick puts every idle villager to work
      (`orders/start_work.rs`).
   3. The player's unit groups update (`groups/update/`).
   4. The player's own update runs: the per-second economy, the trickles, the defeat check
      (`players/world_player/`).
   5. Every live object of the player updates, in list order (`world/object_step.rs`, then
      `Unit::update`). The next section lists what one update does.
   6. Objects that died, fell asleep or changed owner leave the list. The swap-remove of the
      list skips the object that moves into the free place until the next tick, as in the game.
   7. The tiles that the player's sight explored in this tick give it their resources
      (`vision/explored.rs`).
   8. The gates close again.
3. **After the loop.** The world stream draws an intermediate seed. Every player's fog ghosts
   update (`vision/ghost.rs`). All streams take the intermediate seed. The turn passes to the
   next player, the victory check runs (`players/game_status/`), and the world stream draws the
   next frame seed. Last, the players' vision masks are rebuilt (`vision/`).

Only one player's unit AIs run their full update in a tick: the player whose turn it is. The
other players' unit AIs only take their orders and notifications.

## One object's update

`Unit::update` (`objects/unit/update.rs`) is the object's update. It follows the game's chain of
classes: the static object, the animated object, the moving object, the action object, the combat
object and the building.

1. A building's gate, its completion and its seconds (`construction/building_update.rs`).
2. The attack timer and the volley under way run down (`combat/attack/unit.rs`).
3. A moving object takes its velocity step: it moves along its forward vector by its speed
   (`movement/unit.rs`).
4. The life cycle steps: a dead object plays its dying sprite, leaves its remains and goes
   (`objects/unit/life_step.rs`, `objects/lifecycle/`).
5. The sprite's clock goes on (`objects/sprite.rs`).
6. An idle unit starts its master's default task (`objects/unit/default_task.rs`).
7. The action list runs its first action. Each action's part of the update is a method in the
   action's system: the Artifact in `ownership/artifact.rs`, the Build and the Repair in
   `construction/build/unit.rs` and `construction/repair/unit.rs`, the inside update and the Enter
   in `garrison/unit.rs`, the `MoveTo`, the Explore and the Fly in `movement/unit.rs`, the Make in
   `gathering/make.rs`, the Gather in `gathering/gather/unit.rs`, the Hunt in
   `gathering/hunt/hunter.rs` and the Attack in `combat/attack/unit.rs`. A walking action calls the motion step, which plans the path
   (`pathing/`) and steers the unit along it (`movement/`).
8. A combat object's passive abilities run (`combat/abilities/`), and a building's production
   (`production/`).
9. The unit AI runs (`unit_ai/`).

The update does not change the world directly. It reads and writes through a `Context`
(`world/context.rs`), and `World::finish_object` applies what it collected: new objects, kills,
resources, ownership changes.

## The systems

Each top-level folder is one system. The table names the game's classes in it and the game's
source files it comes from. The file names are those that the HD edition's debug symbols name
(8.3 names such as `act_atak.cpp`). They are matched by their names only.

| Folder | What it holds | The game's classes | The game's files |
| --- | --- | --- | --- |
| [`world/`](../crates/engine/genie-core/src/world/) | The world's state and its tick: the setup from a recording's header, the world update, a player, one object's step and its context, the game options, the checkpoint block | `BaseWorld`, `TRIBE_World`, `WorldPlayerBase` | `world.cpp`, `tworld.cpp`, `World.cpp`, `BaseWorld.cpp`, `WorldPlayerBase.cpp` |
| [`objects/`](../crates/engine/genie-core/src/objects/) | One object and what every object runs: its update, its actions' stops, its life and death, its sprite and facing, its masters | `RGE_Static_Object`, `RGE_Animated_Object`, `RGE_Moving_Object`, `RGE_Action_Object`, `RGE_Master_*` | `stat_obj.cpp`, `ani_obj.cpp`, `act_obj.cpp`, `act_list.cpp`, `sprite.cpp`, `asprite.cpp`, `MasterStaticObject.cpp` |
| [`commands/`](../crates/engine/genie-core/src/commands/) | The players' commands and their dispatch: placements, walls, queues, deletes, GAME | `RGE_Command`, `TRIBE_Command` | `command.cpp`, `tcommand.cpp` |
| [`orders/`](../crates/engine/genie-core/src/orders/) | The orders a command gives its units: INTERACT and its splitter, moves, patrols, follows, work and gather orders, the town bell, the Empire Wars start | (in `TRIBE_Command` and the unit AI) | (in `command.cpp` and the unit AI's files) |
| [`unit_ai/`](../crates/engine/genie-core/src/unit_ai/) | The unit AI: its orders and notifies, its target choice, its idle checks, the task finder and its view of the world | `UnitAIModule` and its types, `RGE_Task_List`, `TRIBE_Task_List` | `UnitAIModule.cpp`, `aiuaimod.cpp`, `taiuaimd.cpp`, `task.cpp`, `task_lst.cpp`, `ttsk_lst.cpp` |
| [`groups/`](../crates/engine/genie-core/src/groups/) | Unit groups and their formations, their motion and their members' moves to their slots | `BUnitGroup`, `BFormation`, `BMotion` | `group.cpp`, `formtn.cpp`, `motion.cpp` |
| [`pathing/`](../crates/engine/genie-core/src/pathing/) | Finding a way: the tile pathfinder, the long pather, the short pather, the old hull pather, the obstruction mesh, the zones | the planners, `MShortPather`, `RGE_Zone_Map` | `path.cpp`, `pathsys.cpp`, `mpathsys.cpp`, `zone_map.cpp` |
| [`movement/`](../crates/engine/genie-core/src/movement/) | Walking the way: the velocity step, the motion step, waypoints, and the actions that are walks (`MoveTo`, Explore, a bird's Fly) | `RGE_Moving_Object`, `RGE_Action_Move_To`, `RGE_Action_Explore`, the Fly action (type 10) | `MovingObject.cpp`, `motion.cpp`, `waypoint.cpp`, `act_move.cpp`, `act_expl.cpp`, `act_bird.cpp` |
| [`map/`](../crates/engine/genie-core/src/map/) | The map: the terrain per tile and where the objects stand | `RGE_Map`, the obstruction manager | `map.cpp`, `tmap.cpp` |
| [`vision/`](../crates/engine/genie-core/src/vision/) | What each player sees: the visible maps, the vision masks, the objects-in-sight registry, the fog's ghosts, the explored tiles | `RGE_Visible_Map`, `RGE_Unified_Visible_Map`, `Visible_Unit_Manager`, `RGE_Doppleganger_Object` | `visible.cpp`, `vis_unit.cpp`, `VisibleUnitManager.cpp`, `dpl_obj.cpp` |
| [`combat/`](../crates/engine/genie-core/src/combat/) | Attacks, hits and missiles: the Attack action, damage, passive abilities, missiles and volleys, a hit's victims | `RGE_Action_Attack`, `RGE_Missile_Object`, `RGE_Action_Missile`, `TRIBE_Combat_Object` | `act_atak.cpp`, `act_misl.cpp`, `misl_obj.cpp`, `t_c_obj.cpp`, `TribeMasterCombatObject.cpp` |
| [`gathering/`](../crates/engine/genie-core/src/gathering/) | Gathering resources: the Gather and Hunt actions, a farm's food, the resource searches, drop sites, the mill's queues, pastures, the mule cart | `RGE_Action_Gather`, `TRIBE_Action_Hunt`, `RGE_Action_Make` | `act_gath.cpp`, `tact_gth.cpp`, `tact_hnt.cpp`, `act_make.cpp`, `tact_she.cpp` |
| [`construction/`](../crates/engine/genie-core/src/construction/) | Building and repairing: the Build and Repair actions, the wait for a foundation, a building's own update, the gates | `TRIBE_Action_Build`, `TRIBE_Action_Repair`, `TRIBE_Action_Wait_Solidify`, `TRIBE_Building_Object` | `tact_bld.cpp`, `tact_rep.cpp`, `t_b_obj.cpp` |
| [`garrison/`](../crates/engine/genie-core/src/garrison/) | Units inside other objects: garrisons, entering, unloading, transports | `RGE_Action_Enter`, `RGE_Action_Transport` | `act_entr.cpp`, `act_tran.cpp` |
| [`production/`](../crates/engine/genie-core/src/production/) | A building's production queue and a trained unit's spawn | `TRIBE_Action_Make_Obj`, `TRIBE_Action_Make_Tech` | `t_b_obj.cpp`, `tact_mob.cpp` |
| [`technology/`](../crates/engine/genie-core/src/technology/) | Technologies, their effects, and an object's upgrade to another master | `TRIBE_Player_Tech`, `TRIBE_Effects` | `tech.cpp`, `techtree.cpp`, `effects.cpp`, `teffects.cpp` |
| [`ownership/`](../crates/engine/genie-core/src/ownership/) | Objects that change hands: captures, the Artifact action of livestock, ownership changes | `TRIBE_Action_Artifact` | `tact_cap.cpp`, `tact_art.cpp`, `tact_cnv.cpp` |
| [`players/`](../crates/engine/genie-core/src/players/) | The players beside their objects: game status, diplomacy, tribute, treason, the market, history, the object tracker, the world player's update | `WorldPlayer`, `WorldPlayerGaia`, `TRIBE_History_Info` | `player.cpp`, `tplayer.cpp`, `t_histry.cpp`, `victory.cpp`, `tvictory.cpp`, `VictoryConditions.cpp` |
| [`math/`](../crates/engine/genie-core/src/math/) | The bit-for-bit foundations: the game's f32 math, its random generators, the MSVC algorithms whose order shows, the image's constants | the random generators, `BVector` | `math3d.cpp`, the C runtime |

`lib.rs` declares the systems. `trace.rs` holds what the engine tells a
[`tracing`](https://docs.rs/tracing) subscriber while it runs.

## How the systems depend on each other

The systems do not form strict layers. Two of them are hubs that call into every behaviour:
`world` runs the tick, and `objects` runs an object's update. A `Unit` holds the state of each
kind of action, so `objects` names every system that has an action.

A behaviour that needs the whole world is a method of `World`, and it lives in its own system, not
in `world/`. For example, `World::upgrade_object` is in `technology/upgrade.rs`. So `world/`
holds only the world's state and the tick, and most systems name `crate::world`.

At the bottom, `math` names no other system, and `pathing` names only `map` and `math`.
`map/spatial/` answers the queries of many systems, so it names their types.

Each folder's `README.md` lists the systems that its code names and the systems that name it.
`scripts/readmes.py` generates the lists from the code. Comments and test files do not count.

## Finding a behaviour

- **By system.** Start at the system's folder and its `README.md`. The table of files gives the
  first sentence of each file's documentation.
- **By the game's name.** Every port names the game's routine in its documentation: a class and
  method where the debug symbols give one (`RGE_Action_Attack::work`), else the routine's address
  in build 158041 (`0x140a82ae0`). Search for the name or the address:
  `grep -rn "0x140a82ae0" crates/engine/genie-core/src`.
- **By a stop.** A stop names the missing part in its reason. Search for the reason's text: every
  `GenieError::unsupported("…")` holds it.
- **By a trace.** `trace.rs` lists the targets that the engine logs. [DEBUGGING.md](DEBUGGING.md)
  shows how to play a recording and find where it goes wrong.

## Determinism

genie must give the same bits as the game on every host, WebAssembly included. Four parts of the
code keep it so:

- **The math** (`math/fmath/`). The game's `f32` routines (`sinf`, `atan2f`, `powf`, …) are
  ported from its machine code, with their constants and their operation order. A sum or a
  product is rounded on its own, never fused. A float becomes an integer as the x86 instruction
  does it (`math/conversions.rs`), not as Rust's `as` does.
- **The random streams** (`math/rng.rs`). The game has eleven `WELL512a` streams. All take the
  frame seed at the start of every tick, so the draws of one tick do not reach the next. A port
  draws from the same stream as the game, in the same order.
- **The order of lists.** The objects update in their list's order. A removal swaps the last
  object into the free place, as the game's list does. A sort runs MSVC's introsort
  (`math/msvc.rs`), because the order of equal elements depends on the algorithm.
- **The game data.** The values come from the recording's save and the game's `.dat` file, never
  from constants in the code. `math/image.rs` holds the constants of the game's image that the
  routines read. A script generates it.

## Conventions

- **Names.** Folders and files have plain English names for what they hold. Game addresses and
  class abbreviations stay in the documentation, not in paths or identifiers.
- **One routine, one port.** A port follows its routine whole: every branch, its order of
  operations, its random draws. A part that is not ported yet returns
  `GenieError::unsupported`, and the player stops there.
- **File length.** A hand-written file holds at most 700 lines of production code
  (`scripts/file_size.py`). Tests do not count.
- **An action's folder.** An action has a folder of its own in its system, for example
  `gathering/hunt/`. `mod.rs` holds the action's state and its constants. The other files follow
  the action's slots and states: its start, its update, its `set_state`. `unit.rs` is the
  unit's part of the update, which binds the action to the world. `unit_ai_side.rs` holds what
  the unit AI asks of the action. `tests.rs` holds the action's tests.
- **The folder READMEs.** The prose is written by hand. `scripts/readmes.py SRC` fills the
  generated sections (the table of files, the systems used) between their markers.
  `scripts/readmes.py SRC --check` fails on a missing or stale README. The pre-commit hook and CI
  run the check.
