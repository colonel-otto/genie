//! A combat object's passive abilities: `FUN_1412072f0`, which `TRIBE_Combat_Object::update`
//! (`0x141208440`) runs on every combat unit and building (types 70 and 80) after
//! `RGE_Combat_Object::update`, with what that one returned (the object leaves its list); a
//! building's update (`TRIBE_Building_Object::update`) runs its production after it. The routine
//! walks the object's master's task list (`+0x788`) in order and acts on four action types
//! (`+0xe`):
//!
//! - 151 ([`GENERATE`]): the player's attribute named by the task's `attribute_type3` (`+0x24`)
//!   grows by the world's dt times the task's `work_val1` times the player's attribute named by
//!   its `attribute_type2` (`+0x22`), through the player's store of what it made and did not give
//!   yet (`player+0x658`, [`crate::turn::Player::generated`]): once that reaches 1 it is given
//!   (the player's slot 0xf8) and starts again at 0. A combat unit (or building) with the task's
//!   `combat_level_flag` bit 1 (bit 2) generates always; else a combat unit whose top action
//!   works (its state 7) on a target of the task's class, and a built building. The Feitoria,
//!   the castles, the King of the Hill monument, the Keshik and some villagers have it.
//! - 152 ([`MOVEMENT_DAMAGE`]): while it moves, a combat unit hits the enemies about it on a
//!   timer (`+0x9a4`, `mMovementDamageTimer`): each on the tiles within the task's `work_range`
//!   that is not its player's ally nor Gaia's and is close enough loses the task's `work_val1`
//!   hit points (its slot 0xb0), the timer takes the task's `work_val2` and the hit's player
//!   hears of it (its slot 0x2e8 with 0x201); one left below 1 hit point has 0.
//! - 159 ([`TERRAIN_LOOK`]): the object takes the task's look while it stands on a terrain of
//!   the task's ([`terrain_mask`]): its own sprites (`+0x8a9` set, `+0x960`..`+0x988`, [`Look`]),
//!   which its sprite slots then show ([`crate::sprite::standing`] and the others), its walk's
//!   distance factor (`+0x878`, the task's `work_val1`) and its attack timer's rate (`+0x87c`,
//!   its `work_val2`); off it, its master's and 1.
//! - 161 ([`REMOVAL_REWARD`], `FUN_14120b5f0`): a dead object that leaves its list issues its
//!   player an `ADD_ATTRIBUTE` command for each such task within 1e-7 of range ([`Reward`]).
//!
//! The `.dat` of build 158041 has only 151 tasks (its current build adds 159 for the longboats,
//! Nessie and the snake), and the civilisations' bonuses add more to their gatherers' masters,
//! which the save then holds (`mObjectTaskFlags` bit 0): the Portuguese foragers' wood, the
//! Poles' stone miners' gold, the Shu lumberjacks' food. The routine runs whatever the masters
//! hold.

use crate::damage::Victim;
use crate::fmath;
use crate::masters::Task;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// The action type of a generation task (151): the Feitoria's trickle and the like.
pub const GENERATE: i16 = 0x97;
/// The action type of a movement damage task (152).
pub const MOVEMENT_DAMAGE: i16 = 0x98;
/// The action type of a terrain look task (159).
pub const TERRAIN_LOOK: i16 = 0x9f;
/// The action type of a removal reward task (161).
pub const REMOVAL_REWARD: i16 = 0xa1;

/// The object types the routine runs on: a combat unit (`'F'`) and a building (`'P'`).
const COMBAT_KIND: u8 = 70;
const BUILDING_KIND: u8 = 80;

/// The top action types a generation task's test passes (`FUN_140a7be70`): the Attack (9),
/// type 7, the Gather (5) and the Hunt (0x6e).
const WORKING_ACTIONS: [i16; 4] = [9, 7, 5, 0x6e];
/// The action state of an action at work (`+0x18`).
const WORKING: u8 = 7;

/// The building classes a movement damage hits (`0x141207ae8`, decoded 0x33 and 0x36); it hits
/// any object of another type.
const STRUCK_BUILDING_CLASSES: [i16; 2] = [0x33, 0x36];
/// The blast level from which an object is reached whatever the hitter's level (`+0x62b`).
const ALWAYS_REACHED: u8 = 2;

/// Hit points below which an object counts dead, and the store's give threshold
/// (`DAT_14330eb8c`).
const ONE: f32 = crate::image::abilities::ONE;
/// The range below which a removal reward task pays (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::abilities::EPSILON;

/// What the routine keeps on its object. The constructor's values are the default: no timer,
/// rate 1 and the master's look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Abilities {
    /// `+0x9a4`, `mMovementDamageTimer` (saved): the seconds before a movement damage task hits
    /// again.
    pub movement_damage_timer: f32,
    /// `+0x87c`: the rate the attack timer (`+0x91c`) runs down at (`RGE_Combat_Object::update`,
    /// `0x14107e28e`).
    pub attack_rate: f32,
    /// `+0x8a9` set, with the sprites at `+0x960`..`+0x988`: a terrain look task's look; None
    /// while the object shows its master's (not saved: the constructor clears `+0x8a9`).
    pub look: Option<Look>,
}

impl Default for Abilities {
    fn default() -> Self {
        Self {
            movement_damage_timer: 0.0,
            attack_rate: 1.0,
            look: None,
        }
    }
}

/// The sprites a terrain look task gives its object (graphic ids; None for a null sprite), which
/// the object's sprite slots show while `+0x8a9` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// `+0x960`, the task's carrying sprite (`+0x58`): slot 0x798's standing sprite.
    pub standing: Option<i16>,
    /// `+0x968`, the task's moving sprite (`+0x40`): slot 0x7c8's walking sprite.
    pub walking: Option<i16>,
    /// `+0x970`, the task's working sprite (`+0x50`): slot 0x898's attack sprite.
    pub attacking: Option<i16>,
    /// `+0x978`, the task's proceeding sprite (`+0x48`): slot 0x7d0's running sprite.
    pub running: Option<i16>,
    /// `+0x980`, the sprite the task's gathering sound event id names (`+0x60`): slot 0x7a0's
    /// dying sprite.
    pub dying: Option<i16>,
    /// `+0x988`, the sprite its deposit sound event id names (`+0x78`): slot 0x7a8's undead
    /// sprite.
    pub undead: Option<i16>,
}

/// The `ADD_ATTRIBUTE` command a removal reward task issues (`FUN_14120b5f0`: the player's slot
/// 0x360, `FUN_141616500` -> `FUN_140e618e0`, command type 5 into the command queue,
/// `FUN_140e6e180`): the attribute (its low byte goes into the command) and the amount, which
/// the command carries as a float.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reward {
    /// The player the command is for (`player+0x718`).
    pub player: usize,
    /// The task's `attribute_type3` (`+0x24`).
    pub attribute: i16,
    /// The amount, truncated (`CVTTSS2SI`).
    pub amount: i32,
}

/// The routine's test of the object's hit points (`COMISS 1.0, hp` / `JBE`): at least 1 (or a
/// NaN) takes the living path, else the dead one.
#[must_use]
pub fn alive(hp: f32) -> bool {
    fmath::at_most_or_unordered(ONE, hp)
}

/// What a generation task's test reads of the object's top action (`FUN_140a986f0`): its type
/// (`+0x8`), its state (`+0x18`) and its target as the world holds it (`FUN_140a78450`), None
/// when the world holds no object of its id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopWork {
    /// The action's type.
    pub kind: i16,
    /// The action's state.
    pub state: u8,
    /// Its target.
    pub target: Option<WorkTarget>,
}

/// What a generation task's test reads of the top action's target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkTarget {
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its master's first storage amount (`+0x610`).
    pub storage: f32,
    /// Its master's class (`+0x410`).
    pub class: i16,
}

/// Whether a generation task generates this update (`0x141207d4d`..`0x141207fbe`): an object
/// of type 70 (`+0xf`) whose task has `combat_level_flag` bit 1, or of type 80 with bit 2, does;
/// otherwise one whose master is of type 70 does while its top action is one of
/// [`WORKING_ACTIONS`] in state 7 with a target the world holds that has hit points above 0 or
/// whose master stores some (`COMISS`/`JA`, `JBE`: a NaN does not) and whose master's class is
/// the task's `object_group`, and one whose master is of type 80 does while it is built
/// (`+0xca1` 1).
#[must_use]
pub fn generates(
    task: &Task,
    kind: u8,
    master_kind: u8,
    top: Option<TopWork>,
    built: bool,
) -> bool {
    match kind {
        COMBAT_KIND if task.combat_level_flag & 2 != 0 => return true,
        BUILDING_KIND if task.combat_level_flag & 4 != 0 => return true,
        _ => {}
    }
    if master_kind == COMBAT_KIND
        && let Some(top) = top
        && WORKING_ACTIONS.contains(&top.kind)
        && top.state == WORKING
        && let Some(t) = top.target
        && (0.0 < t.hp || 0.0 < t.storage)
        && t.class == task.object_group
    {
        return true;
    }
    master_kind == BUILDING_KIND && built
}

/// What a generation task makes in an update (`0x141207fc4`): the world's dt (`world+0x164`)
/// times the task's `work_val1` times the player's attribute the task's `attribute_type2` names,
/// in that order (`MULSS`, `MULSS`).
#[must_use]
pub fn generated(dt: f32, task: &Task, multiplier: f32) -> f32 {
    dt * task.work_val1 * multiplier
}

/// The player's store of what its generation tasks made (`player+0x658`): `amount` above 0 (not
/// a NaN) joins the store of attribute `out` when that is not negative (`ADDSS`: the amount plus
/// the store); the store, or the amount alone when it did not join, once at least 1
/// (`COMISS`/`JC`: a NaN is below) goes to the player (returned, the caller's slot 0xf8) and the
/// store starts again at 0. A store the player does not have (an attribute past its count, which
/// no `.dat` task names) takes nothing.
pub fn store(amount: f32, out: i16, stores: &mut [f32]) -> Option<f32> {
    let slot = usize::try_from(out).ok();
    let mut amount = amount;
    if 0.0 < amount
        && let Some(s) = slot.and_then(|i| stores.get_mut(i))
    {
        *s += amount;
        amount = *s;
    }
    if fmath::below_or_unordered(amount, ONE) {
        return None;
    }
    let s = slot.and_then(|i| stores.get_mut(i))?;
    *s = 0.0;
    Some(amount)
}

/// A movement damage task's timer (`+0x9a4`, `0x141207758`): above 0 (not a NaN) it runs down by
/// the world's dt, never below 0. The task hits once it is at most 0 (`COMISS 0, t` / `JC`: a
/// NaN never hits).
pub fn movement_damage_due(timer: &mut f32, dt: f32) -> bool {
    let mut t = *timer;
    if 0.0 < t {
        t -= dt;
        *timer = t;
        if t < 0.0 {
            *timer = 0.0;
            t = 0.0;
        }
    }
    !fmath::below_or_unordered(0.0, t)
}

/// The tiles a movement damage task reaches about the object at `position` (`0x1412077e1`): the
/// truncated range (`CVTTSS2SI`) each way from its truncated place, from 0 (a negative result
/// is 0, `CMOVS`) to the map's width and height (`CMOVL`; both included). Returns the low and
/// the high corner.
#[must_use]
pub fn reach(position: [f32; 3], range: f32, map: [i32; 2]) -> ([i32; 2], [i32; 2]) {
    let r = crate::checkpoint::cvttss2si(range);
    let low = |v: f32| {
        let d = crate::checkpoint::cvttss2si(v).wrapping_sub(r);
        if d < 0 { 0 } else { d }
    };
    let high = |v: f32, size: i32| {
        let d = crate::checkpoint::cvttss2si(v).wrapping_add(r);
        if size < d { size } else { d }
    };
    (
        [low(position[0]), low(position[1])],
        [high(position[0], map[0]), high(position[1], map[1])],
    )
}

/// What a movement damage reads of an object on a tile it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bystander {
    /// The object.
    pub id: i32,
    /// Its player (`+0x138`'s `+0x718`).
    pub player: usize,
    /// The hitter's player counts it allied (`FUN_141663d90` with the ally flag).
    pub allied: bool,
    /// Its type (`+0xf`).
    pub kind: u8,
    /// Its master's class (`+0x410`).
    pub class: i16,
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its master's `area_effect_object_level` (`+0x62b`).
    pub blast_defense: u8,
    /// Its place (`+0x3c8`, `+0x4c8`).
    pub position: [f32; 3],
    /// Its master's radii (`+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
}

/// What a movement damage reads of its hitter: its id, its master's `area_effect_level`
/// (`+0x830`), its place and its master's radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hitter {
    /// The object.
    pub id: i32,
    /// Its master's `area_effect_level` (`+0x830`).
    pub blast_level: u8,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
}

/// Whether a movement damage of `range` hits `b` (`0x141207a29`..`0x141207c0d`): not the hitter
/// itself, not allied nor Gaia's, a building only of class 51 or 54, in state 2 at most, reached
/// by the hitter's level (its own level 2 or more, or the hitter's at most it), and its
/// footprint gap squared (`FUN_14118f8c0`) not past the range squared (`COMISS`/`JA`: a NaN is
/// within).
#[must_use]
pub fn struck(hitter: &Hitter, range: f32, b: &Bystander) -> bool {
    if b.id == hitter.id || b.allied || b.player == 0 {
        return false;
    }
    if b.kind == BUILDING_KIND && !STRUCK_BUILDING_CLASSES.contains(&b.class) {
        return false;
    }
    if 2 < b.state {
        return false;
    }
    if !(ALWAYS_REACHED <= b.blast_defense || hitter.blast_level <= b.blast_defense) {
        return false;
    }
    let d2 = crate::movement::footprint_gap_squared(
        hitter.position,
        hitter.radius,
        b.position,
        b.radius,
    );
    fmath::at_most_or_unordered(d2, range * range)
}

/// The hit's end on an object left below 1 hit point (`COMISS 1.0, hp` / `JBE`: a NaN is not):
/// it has 0, and holds 0 (`FUN_14119d320` with 0, 0 and 1: at most its master's capacity, a
/// capacity below 0 taken as it is). Returns whether it was.
pub fn finish(v: &mut Victim) -> bool {
    if v.hp < ONE {
        v.hp = 0.0;
        v.amount = 0.0;
        let capacity = f32::from(v.capacity);
        if capacity < v.amount {
            v.amount = capacity;
        }
        return true;
    }
    false
}

/// A terrain look task's terrain mask (`0x141207501`, a `std::bitset<8>`): a terrain below 0
/// is the mask of its negation's low byte, 1 to 8 sets the bit of that less 1, above 8 none.
/// None for 0: the bit's `set` throws (`out_of_range`, `FUN_14120b5d0`), which the game does not
/// catch; no task of the `.dat` has terrain 0.
#[must_use]
pub const fn terrain_mask(terrain: i16) -> Option<u8> {
    match terrain {
        t if t < 0 => Some(t.wrapping_neg().to_le_bytes()[0]),
        1..=8 => Some(1 << (terrain - 1)),
        0 => None,
        _ => Some(0),
    }
}

/// The look a terrain look task gives on a tile whose terrain's flag byte (the terrain table's
/// `+0x2`, [`crate::terrain::TerrainKind::flags`]) has a bit of its mask (`0x14120754e`): its
/// sprites, each a null sprite where the world's sprite table (`world+0xc0`) has none
/// (`exists`): the task's ids, −1 for none, its load resolved, and its sound event ids
/// (`FUN_141533c70`: from 0 to the table's count, and not null). A task's id below −1 would
/// read before the table at its load; no `.dat` task has one. None off its terrains.
#[must_use]
pub fn look(task: &Task, mask: u8, flags: u8, exists: impl Fn(i32) -> bool) -> Option<Look> {
    if flags & mask == 0 {
        return None;
    }
    let sprite = |id: i32| i16::try_from(id).ok().filter(|&i| 0 <= i && exists(id));
    let task_sprite = |id: i16| (id != -1).then_some(id).and_then(|id| sprite(id.into()));
    let [moving, proceeding, working, carrying] = task.sprites;
    Some(Look {
        standing: task_sprite(carrying),
        walking: task_sprite(moving),
        attacking: task_sprite(working),
        running: task_sprite(proceeding),
        dying: sprite(task.sound_events[0]),
        undead: sprite(task.sound_events[1]),
    })
}

/// A removal reward task's command (`FUN_14120b5f0`): the task's `work_val1`, times the player's
/// attribute its `attribute_type2` names when that is not negative and the player has it
/// (`FUN_1416625b0`: `multiplier`); with the task's `combat_level_flag` set, the object's master
/// cost (the player's master of its id, its three entries at `+0x880`: resource, amount, paid)
/// of the task's `attribute_type3` decides instead: the amount of the last paid entry of that
/// resource times the rate (`CVTDQ2PS`, `MULSS`), 0 without one. The amount is truncated
/// (`CVTTSS2SI`).
#[must_use]
pub fn reward(
    task: &Task,
    player: usize,
    multiplier: Option<f32>,
    cost: &[(i16, i16, bool)],
) -> Reward {
    let out = task.attribute_type3;
    let mut rate = task.work_val1;
    if 0 <= task.attribute_type2
        && let Some(m) = multiplier
    {
        rate *= m;
    }
    let mut amount = rate;
    if task.combat_level_flag != 0 {
        amount = 0.0;
        for &(resource, count, paid) in cost {
            if resource == out && paid {
                amount = f32::from(count) * rate;
            }
        }
    }
    Reward {
        player,
        attribute: out,
        amount: crate::checkpoint::cvttss2si(amount),
    }
}

/// The action types the routine acts on.
pub const ACTIONS: [i16; 4] = [GENERATE, MOVEMENT_DAMAGE, TERRAIN_LOOK, REMOVAL_REWARD];

/// What the routine reads of the object's master (`+0xa0`: its own one, else its player's).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Own {
    /// The master's id (`+0x58`).
    id: i16,
    /// Its type (`+0x8`).
    kind: u8,
    /// Its `area_effect_level` (`+0x830`).
    blast_level: u8,
    /// Its radii (`+0x5d8`, `+0x5dc`).
    radius: [f32; 2],
}

/// The objects of the world a movement damage may reach whose update is still to write them
/// back (the caller's): the target of the hitter's own attack and the objects of its blast.
#[derive(Debug)]
pub struct Pending<'a> {
    /// The target of its attack, read before its update.
    pub victim: Option<&'a mut Victim>,
    /// The objects its blast reached.
    pub blasted: &'a mut [Victim],
}

impl Pending<'_> {
    /// The pending object `id`.
    fn get(&mut self, id: i32) -> Option<&mut Victim> {
        if let Some(v) = self.victim.as_deref_mut().filter(|v| v.id == id) {
            return Some(v);
        }
        self.blasted.iter_mut().find(|v| v.id == id)
    }
}

impl Turns {
    /// `FUN_1412072f0` on the object `id`, the one at `i` of player `p`'s list, as player
    /// `owner`'s (its owner after its update: the new one when the update handed it over),
    /// once its combat update returned `leaves`. Only its master's tasks of the four types
    /// count ([`ACTIONS`]); an object of a master of another type than 70 or 80, or without one
    /// the slice has, has none. On a living object ([`alive`]) they act in their order
    /// ([`Turns::generate`], [`Turns::movement_damage`], [`Turns::terrain_look`]); on a dead one
    /// that leaves, its removal reward tasks within 1e-7 of range (`COMISS`/`JBE`: a NaN is not)
    /// issue their commands, which are returned: in a recording's replay the recording's own
    /// commands bring them.
    ///
    /// # Errors
    ///
    /// What the slice does not model of a task's action ([`Turns::generate`],
    /// [`Turns::movement_damage`]).
    pub(crate) fn abilities(
        &mut self,
        (id, p, i): (i32, usize, usize),
        owner: usize,
        leaves: bool,
        pending: &mut Pending<'_>,
    ) -> Result<Vec<Reward>, Unsupported> {
        let i = match self.players[p].objects.get(i) {
            Some(u) if u.id == id => i,
            _ => match self.place(id) {
                Some(crate::order::Place::Live(q, j)) if q == p => j,
                _ => return Ok(Vec::new()),
            },
        };
        let unit = &self.players[p].objects[i];
        let Some(master) = unit
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(p).and_then(|m| m.get(unit.life.master_id)))
            .filter(|m| matches!(m.kind, COMBAT_KIND | BUILDING_KIND))
        else {
            return Ok(Vec::new());
        };
        let tasks: Vec<Task> = master
            .tasks
            .iter()
            .filter(|t| ACTIONS.contains(&t.action_type))
            .copied()
            .collect();
        if tasks.is_empty() {
            return Ok(Vec::new());
        }
        let own = Own {
            id: master.id,
            kind: master.kind,
            blast_level: master.blast_level,
            radius: master.radius,
        };
        let mut rewards = Vec::new();
        if !alive(unit.life.hp) {
            if !leaves {
                return Ok(rewards);
            }
            for task in &tasks {
                if task.action_type == REMOVAL_REWARD && task.work_range < EPSILON {
                    let multiplier = usize::try_from(task.attribute_type2)
                        .ok()
                        .and_then(|a| self.players[owner].attribute(a));
                    let cost = self
                        .masters
                        .get(owner)
                        .and_then(|m| m.get(own.id))
                        .map_or(&[][..], |m| &m.cost[..]);
                    rewards.push(reward(task, owner, multiplier, cost));
                }
            }
            return Ok(rewards);
        }
        for task in &tasks {
            match task.action_type {
                GENERATE => self.generate((p, i), owner, own, task, pending)?,
                MOVEMENT_DAMAGE => self.movement_damage((p, i), owner, own, task, pending)?,
                // the terrain mask's throw ends the game
                TERRAIN_LOOK if !self.terrain_look((p, i), task) => return Ok(rewards),
                _ => {}
            }
        }
        Ok(rewards)
    }

    /// A generation task of the object at `(p, i)` as `owner`'s ([`generates`], [`generated`],
    /// [`store`]): what reaches the store's threshold goes to the player (slot 0xf8 with 1 and 1,
    /// as a load's, [`crate::turn::give_load`]); the statistics (`FUN_14097daf0`, the local
    /// player's, and `FUN_140a4c020`'s telemetry) are not held. The top action's target is read
    /// as the world holds it after the update: a pending one as the update left it.
    ///
    /// # Errors
    ///
    /// The top action of a unit whose action the slice does not model, and a give to a player
    /// who shares with allies ([`crate::turn::give_load`]).
    fn generate(
        &mut self,
        (p, i): (usize, usize),
        owner: usize,
        master: Own,
        task: &Task,
        pending: &mut Pending<'_>,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[p].objects[i];
        let flags = task.combat_level_flag;
        let short_cut = match master.kind {
            COMBAT_KIND => flags & 2 != 0,
            _ => flags & 4 != 0,
        };
        let top = if short_cut || master.kind != COMBAT_KIND {
            None
        } else {
            if let Some(e) = unit.unmodelled_action {
                return Err(e);
            }
            unit.top_action().map(|a| {
                // FUN_140a78450: an id below 0 names no object; the target only matters to an
                // action of the test's at work
                let works = WORKING_ACTIONS.contains(&a.kind) && a.state == WORKING;
                let target = unit
                    .top_target()
                    .filter(|&t| works && 0 <= t)
                    .and_then(|t| self.work_target(t, pending));
                TopWork {
                    kind: a.kind,
                    state: a.state,
                    target,
                }
            })
        };
        let built = master.kind == BUILDING_KIND
            && self
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(unit.id))
                .and_then(|s| s.building)
                .is_some_and(|b| b.built);
        if !generates(task, master.kind, master.kind, top, built) {
            return Ok(());
        }
        let player = &mut self.players[owner];
        let multiplier = usize::try_from(task.attribute_type2)
            .ok()
            .and_then(|a| player.attribute(a))
            .unwrap_or(0.0);
        let amount = generated(self.dt_seconds, task, multiplier);
        let Some(give) = store(amount, task.attribute_type3, &mut player.generated) else {
            return Ok(());
        };
        let sharing = player.sharing;
        let mut adds = Vec::new();
        crate::turn::give_load(
            (&self.spatial.relations, sharing),
            owner,
            &mut adds,
            task.attribute_type3,
            give,
        )?;
        for (kind, amount) in adds {
            self.players[owner].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// What a generation task's test reads of the object `id` its top action works on: a pending
    /// one as its update left it, else as the world holds it; None for an object the world does
    /// not hold.
    fn work_target(&self, id: i32, pending: &mut Pending<'_>) -> Option<WorkTarget> {
        if let Some(v) = pending.get(id) {
            return Some(WorkTarget {
                hp: v.hp,
                storage: v.storage,
                class: v.class,
            });
        }
        let (q, u) = self.object(id)?;
        let m = u
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(q).and_then(|m| m.get(u.life.master_id)))?;
        Some(WorkTarget {
            hp: u.life.hp,
            storage: m.storage.first().map_or(0.0, |s| s.1),
            class: m.class,
        })
    }

    /// A movement damage task of the object at `(p, i)` as `owner`'s ([`movement_damage_due`]):
    /// once due, a combat unit (its master of type 70) that moves (slot 0x5c8, `FUN_1411583d0`:
    /// its velocity's length above 0) hits each object on the tiles of [`reach`], row by row,
    /// each tile's list in order, that it strikes ([`struck`]): the object loses the task's
    /// `work_val1` (slot 0xb0, [`crate::damage::lose`]), the timer takes the task's `work_val2`,
    /// and one left below 1 hit point ends the hit ([`finish`]): a combat unit (type 70) counts
    /// killed by the owner (`FUN_1412005b0`, [`Turns::count_kill`]). A ghost on a tile is struck
    /// as an object of type 25 of its viewer, of the ghost master, in its state; the hit points
    /// it loses are not held. Not held: the alerts the hit's player hears (its slot 0x2e8 with
    /// 0x201, `FUN_14162dab0`: the human player's alerts and a computer player's AI), as for a
    /// hit ([`crate::damage`]). A reach past the map's last row or column reads a tile the map
    /// does not have (`FUN_140f5ec30`'s null tile, whose count the routine reads: the game
    /// faults); the scan stops there.
    ///
    /// # Errors
    ///
    /// An object on a tile the slice does not hold, and what [`Turns::count_kill`] does not
    /// model.
    fn movement_damage(
        &mut self,
        (p, i): (usize, usize),
        owner: usize,
        master: Own,
        task: &Task,
        pending: &mut Pending<'_>,
    ) -> Result<(), Unsupported> {
        let dt = self.dt_seconds;
        let unit = &mut self.players[p].objects[i];
        if !movement_damage_due(&mut unit.abilities.movement_damage_timer, dt) {
            return Ok(());
        }
        let moving = unit
            .mover
            .as_ref()
            .and_then(|m| m.velocity.as_ref())
            .is_some_and(|v| 0.0 < fmath::length(v.velocity));
        if master.kind != COMBAT_KIND || !moving {
            return Ok(());
        }
        let hitter = Hitter {
            id: unit.id,
            blast_level: master.blast_level,
            position: unit.position,
            radius: master.radius,
        };
        let range = task.work_range;
        let (low, high) = reach(unit.position, range, self.spatial.map);
        for y in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                if x < 0 || y < 0 || self.spatial.map[0] <= x || self.spatial.map[1] <= y {
                    return Ok(());
                }
                for id in self.spatial.tile_list(x, y).to_vec() {
                    let Some(b) = self.bystander(id, owner, pending)? else {
                        continue;
                    };
                    if !struck(&hitter, range, &b) {
                        continue;
                    }
                    self.players[p].objects[i].abilities.movement_damage_timer = task.work_val2;
                    self.movement_hit(b, owner, task.work_val1, pending)?;
                }
            }
        }
        Ok(())
    }

    /// What a movement damage reads of the object `id` on a tile, as player `owner`'s hitter
    /// sees it: a pending object as its hitter's update left it, a ghost as its viewer's object
    /// of the ghost master, any other as the world holds it.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold ([`crate::build::NOT_HELD`]).
    fn bystander(
        &self,
        id: i32,
        owner: usize,
        pending: &mut Pending<'_>,
    ) -> Result<Option<Bystander>, Unsupported> {
        let allied =
            |q: usize| crate::tasks::allied(&self.players[owner].relations, owner, q, true);
        if let Some(v) = pending.get(id) {
            return Ok(Some(Bystander {
                id,
                player: v.player,
                allied: allied(v.player),
                kind: v.kind,
                class: v.class,
                state: v.state,
                blast_defense: v.blast_defense,
                position: v.position,
                radius: v.radius,
            }));
        }
        if let Some((q, u)) = self.object(id) {
            let m = u
                .own_master
                .as_deref()
                .or_else(|| self.masters.get(q).and_then(|m| m.get(u.life.master_id)))
                .ok_or(crate::build::NOT_HELD)?;
            return Ok(Some(Bystander {
                id,
                player: q,
                allied: allied(q),
                kind: m.kind,
                class: m.class,
                state: u.life.state,
                blast_defense: m.blast_defense,
                position: u.position,
                radius: m.radius,
            }));
        }
        for (q, player) in self.players.iter().enumerate() {
            if let Some(g) = player.ghosts.iter().find(|g| g.id == id) {
                let m = self.master_of(q, crate::ghost::GHOST_MASTER)?;
                return Ok(Some(Bystander {
                    id,
                    player: q,
                    allied: allied(q),
                    kind: crate::ghost::GHOST,
                    class: m.class,
                    state: g.state,
                    blast_defense: m.blast_defense,
                    position: g.position,
                    radius: m.radius,
                }));
            }
        }
        Err(crate::build::NOT_HELD)
    }

    /// A movement damage's hit of `b` by player `owner`'s hitter: `damage` hit points off (slot
    /// 0xb0), then [`finish`] and a combat unit's kill count.
    fn movement_hit(
        &mut self,
        b: Bystander,
        owner: usize,
        damage: f32,
        pending: &mut Pending<'_>,
    ) -> Result<(), Unsupported> {
        if b.kind == crate::ghost::GHOST {
            return Ok(());
        }
        if let Some(v) = pending.get(b.id) {
            crate::damage::lose(v, damage);
            if finish(v) && b.kind == COMBAT_KIND {
                self.count_kill(b.id, owner)?;
            }
            return Ok(());
        }
        let Some((q, u)) = self.object(b.id) else {
            return Err(crate::build::NOT_HELD);
        };
        let m = u
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(q).and_then(|m| m.get(u.life.master_id)))
            .ok_or(crate::build::NOT_HELD)?;
        let mut v = crate::turn::victim_of(
            u,
            m,
            &self.players[q].attributes,
            q,
            self.spatial.terrain.as_ref(),
            self.mode_680,
        );
        crate::damage::lose(&mut v, damage);
        let killed = finish(&mut v);
        let (_, u) = self.object_mut(b.id).ok_or(crate::build::NOT_HELD)?;
        u.life.hp = v.hp;
        u.life.amount = v.amount;
        if let Some(a) = v.armored.as_ref() {
            u.hp_powerups = a.hp_powerups;
        }
        if killed && b.kind == COMBAT_KIND {
            self.count_kill(b.id, owner)?;
        }
        Ok(())
    }

    /// A terrain look task of the object at `(p, i)` (`0x141207501`): its walk's distance factor
    /// (`+0x878`, [`crate::movement::Mover::factor`]) and its attack timer's rate (`+0x87c`) go
    /// back to 1 and its look to its master's, then on a tile of the task's terrains
    /// ([`terrain_mask`], [`look`]) take the task's `work_val1`, `work_val2` and look. The tile
    /// is the one under where the object last moved to (slot 0x520: `+0x824`, `+0x82c`,
    /// truncated; a building's place); without the map's terrain no tile matches. False when
    /// the task's terrain mask throws ([`terrain_mask`]): the routine ends.
    fn terrain_look(&mut self, (p, i): (usize, usize), task: &Task) -> bool {
        let Some(mask) = terrain_mask(task.terrain) else {
            return false;
        };
        let unit = &self.players[p].objects[i];
        let at = unit.mover.as_ref().map_or(unit.position, |m| m.moved_at);
        let (x, y) = (
            crate::checkpoint::cvttss2si(at[0]),
            crate::checkpoint::cvttss2si(at[1]),
        );
        let flags = self.spatial.terrain.as_ref().map_or(0, |t| {
            t.kind(x, y)
                .and_then(|k| t.terrains.get(usize::from(k)))
                .map_or(0, |k| k.flags)
        });
        let graphics = &self.graphics;
        let exists = |id: i32| i16::try_from(id).is_ok_and(|id| graphics.contains_key(&id));
        let look = look(task, mask, flags, exists);
        let unit = &mut self.players[p].objects[i];
        let (factor, rate) = if look.is_some() {
            (task.work_val1, task.work_val2)
        } else {
            (ONE, ONE)
        };
        if let Some(m) = unit.mover.as_mut() {
            m.factor = factor;
        }
        unit.abilities.attack_rate = rate;
        unit.abilities.look = look;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(action_type: i16) -> Task {
        Task {
            action_type,
            ..Task::default()
        }
    }

    /// The Feitoria's first task (1021, task 0): food from attribute 205 at 1.6 a second.
    fn feitoria_food() -> Task {
        Task {
            object_group: -1,
            attribute_type2: 205,
            attribute_type3: 0,
            work_val1: 1.6,
            combat_level_flag: 1,
            ..task(GENERATE)
        }
    }

    /// The Keshik's task 5 (1228): gold from attribute 213 at 0.01 while it fights class 0.
    fn keshik_gold() -> Task {
        Task {
            object_group: 0,
            attribute_type2: 213,
            attribute_type3: 3,
            work_val1: 0.01,
            combat_level_flag: 1,
            ..task(GENERATE)
        }
    }

    #[test]
    fn the_living_path_takes_at_least_1_hit_point_or_a_nan() {
        assert!(alive(1.0));
        assert!(alive(25.0));
        assert!(alive(f32::NAN));
        assert!(!alive(0.999_999_94));
        assert!(!alive(0.0));
    }

    #[test]
    fn a_task_with_its_type_s_flag_bit_generates_always() {
        // bit 1 for an object of type 70, bit 2 for one of type 80
        let mut t = feitoria_food();
        t.combat_level_flag = 2;
        assert!(generates(&t, 70, 70, None, false));
        assert!(!generates(&t, 80, 80, None, false));
        t.combat_level_flag = 4;
        assert!(generates(&t, 80, 80, None, false));
        assert!(!generates(&t, 70, 70, None, false));
    }

    #[test]
    fn a_building_generates_once_built() {
        let t = feitoria_food();
        assert!(!generates(&t, 80, 80, None, false));
        assert!(generates(&t, 80, 80, None, true));
        // a combat unit's master is not a building's
        assert!(!generates(&t, 70, 70, None, true));
    }

    #[test]
    fn a_unit_generates_while_its_top_action_works_on_a_target_of_the_task_s_class() {
        let t = keshik_gold();
        let target = WorkTarget {
            hp: 45.0,
            storage: 0.0,
            class: 0,
        };
        let top = |kind, state, target| {
            Some(TopWork {
                kind,
                state,
                target,
            })
        };
        // the Attack (9), type 7, the Gather (5) and the Hunt (0x6e), in state 7
        for kind in [9, 7, 5, 0x6e] {
            assert!(generates(&t, 70, 70, top(kind, 7, Some(target)), false));
        }
        assert!(!generates(&t, 70, 70, top(1, 7, Some(target)), false));
        assert!(!generates(&t, 70, 70, top(9, 6, Some(target)), false));
        assert!(!generates(&t, 70, 70, top(9, 7, None), false));
        assert!(!generates(&t, 70, 70, None, false));
        // another class
        let other = WorkTarget { class: 6, ..target };
        assert!(!generates(&t, 70, 70, top(9, 7, Some(other)), false));
        // a dead target counts while its master stores some, never with a NaN
        let dead = WorkTarget {
            hp: 0.0,
            storage: 1.0,
            ..target
        };
        assert!(generates(&t, 70, 70, top(9, 7, Some(dead)), false));
        let empty = WorkTarget {
            hp: 0.0,
            storage: 0.0,
            ..target
        };
        assert!(!generates(&t, 70, 70, top(9, 7, Some(empty)), false));
        let nan = WorkTarget {
            hp: f32::NAN,
            storage: f32::NAN,
            ..target
        };
        assert!(!generates(&t, 70, 70, top(9, 7, Some(nan)), false));
    }

    #[test]
    fn the_amount_is_dt_times_the_rate_times_the_attribute_in_that_order() {
        let t = feitoria_food();
        let dt = 0.044_f32;
        assert_eq!(generated(dt, &t, 1.0).to_bits(), (dt * 1.6 * 1.0).to_bits());
        let m = 0.3_f32;
        assert_eq!(generated(dt, &t, m).to_bits(), ((dt * 1.6) * m).to_bits());
        assert_eq!(generated(dt, &t, 0.0), 0.0);
    }

    #[test]
    fn the_store_gives_what_reached_1_and_starts_again() {
        let mut stores = vec![0.0_f32; 4];
        assert_eq!(store(0.6, 0, &mut stores), None);
        assert_eq!(stores[0], 0.6);
        // ADDSS: the amount plus the store
        let sum = 0.5_f32 + 0.6;
        assert_eq!(store(0.5, 0, &mut stores), Some(sum));
        assert_eq!(stores[0], 0.0);
        // nothing above 0 joins: the store stays, and nothing is given
        stores[1] = 0.9;
        assert_eq!(store(0.0, 1, &mut stores), None);
        assert_eq!(store(f32::NAN, 1, &mut stores), None);
        assert_eq!(store(-2.0, 1, &mut stores), None);
        assert_eq!(stores[1], 0.9);
        // a NaN store gives nothing (JC)
        stores[2] = f32::NAN;
        assert_eq!(store(0.5, 2, &mut stores), None);
        // a negative resource: no store and no give, however large
        assert_eq!(store(5.0, -1, &mut stores), None);
    }

    #[test]
    fn the_movement_damage_timer_runs_down_to_0_and_then_hits() {
        let mut t = 0.5_f32;
        assert!(!movement_damage_due(&mut t, 0.2));
        assert_eq!(t, 0.5 - 0.2);
        assert!(movement_damage_due(&mut t, 0.4));
        assert_eq!(t, 0.0);
        // at 0 it does not run down and hits
        assert!(movement_damage_due(&mut t, 0.4));
        assert_eq!(t, 0.0);
        // to exactly 0
        let mut t = 0.25_f32;
        assert!(movement_damage_due(&mut t, 0.25));
        assert_eq!(t, 0.0);
        // below 0 (a timer the save holds so) hits as it is
        let mut t = -1.0_f32;
        assert!(movement_damage_due(&mut t, 0.1));
        assert_eq!(t, -1.0);
        // a NaN never hits and stays
        let mut t = f32::NAN;
        assert!(!movement_damage_due(&mut t, 0.1));
        assert!(t.is_nan());
    }

    #[test]
    fn the_reach_is_the_truncated_range_about_the_truncated_place_within_the_map() {
        assert_eq!(
            reach([10.7, 20.2, 0.0], 2.9, [120, 120]),
            ([8, 18], [12, 22])
        );
        // from 0, and up to the map's size itself
        assert_eq!(
            reach([1.5, 118.9, 0.0], 3.0, [120, 120]),
            ([0, 115], [4, 120])
        );
        // a NaN range is the integer indefinite
        // (5 less it wraps to a negative value, 0; 5 plus it stays below the map's size): no
        // tile
        let (low, high) = reach([5.0, 5.0, 0.0], f32::NAN, [120, 120]);
        assert_eq!(low, [0, 0]);
        assert_eq!(high, [i32::MIN + 5; 2]);
    }

    fn hitter() -> Hitter {
        Hitter {
            id: 1,
            blast_level: 1,
            position: [10.0, 10.0, 0.0],
            radius: [0.4, 0.4],
        }
    }

    fn bystander() -> Bystander {
        Bystander {
            id: 2,
            player: 2,
            allied: false,
            kind: 70,
            class: 6,
            state: 2,
            blast_defense: 1,
            position: [11.5, 10.0, 0.0],
            radius: [0.2, 0.2],
        }
    }

    #[test]
    fn a_movement_damage_strikes_enemies_in_reach() {
        let h = hitter();
        let b = bystander();
        // the gap is 1.5 - 0.6 = 0.9: within 1 (0.81 at most 1), not within 0.5
        assert!(struck(&h, 1.0, &b));
        assert!(!struck(&h, 0.5, &b));
        assert!(struck(&h, f32::NAN, &b));
        assert!(!struck(&h, 1.0, &Bystander { id: 1, ..b }));
        assert!(!struck(&h, 1.0, &Bystander { allied: true, ..b }));
        assert!(!struck(&h, 1.0, &Bystander { player: 0, ..b }));
        assert!(!struck(&h, 1.0, &Bystander { state: 3, ..b }));
        // buildings only of classes 51 and 54
        let building = Bystander {
            kind: 80,
            class: 3,
            ..b
        };
        assert!(!struck(&h, 1.0, &building));
        assert!(struck(
            &h,
            1.0,
            &Bystander {
                class: 51,
                ..building
            }
        ));
        assert!(struck(
            &h,
            1.0,
            &Bystander {
                class: 54,
                ..building
            }
        ));
        // the blast levels
        let high = Hitter {
            blast_level: 3,
            ..h
        };
        assert!(!struck(&high, 1.0, &b));
        assert!(struck(
            &high,
            1.0,
            &Bystander {
                blast_defense: 2,
                ..b
            }
        ));
        assert!(struck(
            &Hitter {
                blast_level: 1,
                ..h
            },
            1.0,
            &Bystander {
                blast_defense: 1,
                ..b
            }
        ));
    }

    #[test]
    fn a_hit_below_1_hit_point_leaves_0_and_holds_nothing() {
        let mut v = Victim {
            hp: 0.5,
            amount: 30.0,
            capacity: 100,
            ..Victim::default()
        };
        assert!(finish(&mut v));
        assert_eq!((v.hp, v.amount), (0.0, 0.0));
        // a capacity below 0 is taken as it is
        let mut v = Victim {
            hp: -3.0,
            amount: 30.0,
            capacity: -1,
            ..Victim::default()
        };
        assert!(finish(&mut v));
        assert_eq!((v.hp, v.amount), (0.0, -1.0));
        let mut v = Victim {
            hp: 1.0,
            amount: 30.0,
            ..Victim::default()
        };
        assert!(!finish(&mut v));
        assert_eq!(v.amount, 30.0);
        let mut v = Victim {
            hp: f32::NAN,
            ..Victim::default()
        };
        assert!(!finish(&mut v));
    }

    #[test]
    fn the_terrain_mask_is_a_bitset_of_8() {
        // the longboats' -32: the mask 32
        assert_eq!(terrain_mask(-32), Some(32));
        assert_eq!(terrain_mask(-3), Some(3));
        // the negation's low byte
        assert_eq!(terrain_mask(-0x103), Some(3));
        assert_eq!(terrain_mask(i16::MIN), Some(0));
        assert_eq!(terrain_mask(1), Some(1));
        assert_eq!(terrain_mask(8), Some(0x80));
        assert_eq!(terrain_mask(9), Some(0));
        assert_eq!(terrain_mask(0), None);
    }

    /// The longboat's task 3 (250 in the current build): terrains of flag 32, sprites 19601,
    /// −1, 19599, 19599, the dying and undead sprites 19598 and 19600.
    fn longboat_look() -> Task {
        Task {
            terrain: -32,
            work_val1: 0.35,
            work_val2: 1.0,
            sprites: [19601, -1, 19599, 19599],
            sound_events: [19598, 19600],
            ..task(TERRAIN_LOOK)
        }
    }

    #[test]
    fn a_terrain_look_gives_the_task_s_sprites_on_its_terrains() {
        let t = longboat_look();
        let all = |_| true;
        assert_eq!(
            look(&t, 32, 0x21, all),
            Some(Look {
                standing: Some(19599),
                walking: Some(19601),
                attacking: Some(19599),
                running: None,
                dying: Some(19598),
                undead: Some(19600),
            })
        );
        assert_eq!(look(&t, 32, 1, all), None);
        // a sprite the table lacks is a null one
        let some = |id: i32| id != 19600 && id != 19599;
        let l = look(&t, 32, 32, some).unwrap();
        assert_eq!((l.standing, l.undead, l.walking), (None, None, Some(19601)));
        // an event id past the table's ids is none
        let t = Task {
            sound_events: [0x62d4_b3ec, -5],
            ..longboat_look()
        };
        let l = look(&t, 32, 32, all).unwrap();
        assert_eq!((l.dying, l.undead), (None, None));
    }

    /// A removal reward task: 10 gold (3) a removal, by attribute 7's rate.
    fn bounty() -> Task {
        Task {
            attribute_type2: 7,
            attribute_type3: 3,
            work_val1: 10.0,
            ..task(REMOVAL_REWARD)
        }
    }

    #[test]
    fn a_removal_reward_pays_the_rate_times_the_attribute() {
        let t = bounty();
        assert_eq!(
            reward(&t, 2, Some(1.5), &[]),
            Reward {
                player: 2,
                attribute: 3,
                amount: 15
            }
        );
        // truncated
        assert_eq!(reward(&t, 2, Some(0.25), &[]).amount, 2);
        // the player lacks the attribute, or the task names none
        assert_eq!(reward(&t, 2, None, &[]).amount, 10);
        let none = Task {
            attribute_type2: -1,
            ..bounty()
        };
        assert_eq!(reward(&none, 2, Some(3.0), &[]).amount, 10);
        // a NaN rate is the integer indefinite
        assert_eq!(reward(&t, 2, Some(f32::NAN), &[]).amount, i32::MIN);
    }

    #[test]
    fn a_removal_reward_with_its_flag_pays_the_last_paid_cost_of_its_resource_times_the_rate() {
        let t = Task {
            combat_level_flag: 1,
            ..bounty()
        };
        let cost = [(3, 20, true), (0, 50, true), (3, 30, false)];
        assert_eq!(reward(&t, 1, Some(0.5), &cost).amount, 100);
        let cost = [(3, 20, true), (3, 30, true)];
        assert_eq!(reward(&t, 1, Some(1.0), &cost).amount, 300);
        assert_eq!(reward(&t, 1, Some(1.0), &[(0, 50, true)]).amount, 0);
    }

    use crate::masters::{Master, Masters};
    use crate::spatial::{Placed, Spatial};
    use crate::turn::{Player, Unit};

    /// Gaia and two enemies, the world's dt half a second, a 20 by 20 map.
    fn world() -> Turns {
        let player = |relations: Vec<u8>| Player {
            attributes: vec![0.0; 600],
            generated: vec![0.0; 600],
            relations,
            ..Player::default()
        };
        let players = vec![
            player(vec![0, 3, 3]),
            player(vec![3, 0, 3]),
            player(vec![3, 3, 0]),
        ];
        let mut t = Turns::new(0, 1, 0, vec![0; 3], players);
        t.dt_seconds = 0.5;
        t.spatial = Spatial::new([20, 20]);
        t.spatial.relations = t.players.iter().map(|p| p.relations.clone()).collect();
        t.masters = vec![Masters::default(), Masters::default(), Masters::default()];
        t
    }

    /// Master `id` of `kind` and `class` with `tasks`, given to every player.
    fn give_master(t: &mut Turns, id: i16, kind: u8, class: i16, tasks: Vec<Task>) {
        let m = Master {
            id,
            kind,
            class,
            tasks,
            radius: [0.5, 0.5],
            hit_points: 100,
            ..Master::default()
        };
        for ms in &mut t.masters {
            let mut all: Vec<Master> = ms.iter().cloned().collect();
            all.push(m.clone());
            *ms = Masters::new(all);
        }
    }

    /// Object `id` of master `m` with `hp` hit points at (`x`, `y`) in player `p`'s list, placed
    /// on its tile.
    fn put(t: &mut Turns, p: usize, id: i32, m: i16, hp: f32, [x, y]: [f32; 2]) {
        let mut u = Unit {
            id,
            position: [x, y, 0.0],
            ..Unit::default()
        };
        u.life.master_id = m;
        u.life.hp = hp;
        u.life.state = 2;
        t.players[p].objects.push(u);
        let radius = t.masters[p].get(m).map_or([0.5; 2], |m| m.radius);
        t.spatial.insert(
            id,
            Placed {
                position: [x, y, 0.0],
                radius,
                class: 0,
                player: p,
                master: m,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
    }

    /// The abilities of object `id` of player `p` as `p`'s, nothing pending, which issue no
    /// command.
    fn run(t: &mut Turns, p: usize, id: i32) {
        assert_eq!(rewards(t, p, id, false), Vec::new());
    }

    /// The abilities of object `id` of player `p` as `p`'s, nothing pending: the commands they
    /// issue.
    fn rewards(t: &mut Turns, p: usize, id: i32, leaves: bool) -> Vec<Reward> {
        let i = t.players[p]
            .objects
            .iter()
            .position(|u| u.id == id)
            .unwrap();
        let mut pending = Pending {
            victim: None,
            blasted: &mut [],
        };
        t.abilities((id, p, i), p, leaves, &mut pending).unwrap()
    }

    /// The players' sight with `id` a building of player `p` that is `built`.
    fn building_sight(t: &mut Turns, p: usize, id: i32, built: bool) {
        let map = || crate::visibility::VisibleMap {
            counts: vec![0; 400],
            explored: 0,
        };
        let mut v = crate::visibility::Visibility::new(
            20,
            20,
            vec![map(), map(), map()],
            vec![0; 400],
            Vec::new(),
            0,
            vec![false; 3],
        );
        v.add_seer(
            id,
            crate::visibility::Seer {
                player: p,
                master: 1021,
                los: 6.0,
                state: 2,
                explored: true,
                building: Some(crate::visibility::BuildingSight {
                    built,
                    started: true,
                }),
                umv: 0,
                capture: 0,
                kind: 80,
            },
        );
        t.spatial.sight = Some(v);
    }

    #[test]
    fn a_built_feitoria_trickles_food_through_its_player_s_store() {
        let mut t = world();
        // its explore task (6) does nothing here
        give_master(&mut t, 1021, 80, 3, vec![task(6), feitoria_food()]);
        put(&mut t, 1, 50, 1021, 2000.0, [5.5, 5.5]);
        building_sight(&mut t, 1, 50, true);
        t.players[1].attributes[205] = 1.0;
        run(&mut t, 1, 50);
        let made = 0.5_f32 * 1.6 * 1.0;
        assert_eq!(t.players[1].generated[0], made);
        assert_eq!(t.players[1].resources[0], 0.0);
        run(&mut t, 1, 50);
        let given = made + made;
        assert_eq!(t.players[1].resources[0], given);
        assert_eq!(t.players[1].attributes[166], given);
        assert_eq!(t.players[1].generated[0], 0.0);
        // unbuilt, nothing
        building_sight(&mut t, 1, 50, false);
        run(&mut t, 1, 50);
        assert_eq!(t.players[1].generated[0], 0.0);
        // the player's attribute 205 at 0: nothing joins the store
        building_sight(&mut t, 1, 50, true);
        t.players[1].attributes[205] = 0.0;
        run(&mut t, 1, 50);
        assert_eq!(t.players[1].generated[0], 0.0);
    }

    /// A Keshik of player 1 attacking object 77 in state 7.
    fn keshik(t: &mut Turns) {
        give_master(t, 1228, 70, 12, vec![keshik_gold()]);
        put(t, 1, 60, 1228, 100.0, [5.5, 5.5]);
        let attack = crate::attack::Attack {
            target: 77,
            target2: -1,
            state: 7,
            timer: 0.0,
            range: 0.0,
            min_range: 0.0,
            point: [-1.0; 3],
            seen: [-1.0; 3],
            move_to: None,
            move_wait: crate::move_to::MoveWait::default(),
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms: crate::attack::Arms {
                move_sprite: None,
                sprite: None,
                second_sprite: None,
                idle_sprite: None,
                idle_attack_sprite: None,
                missile: -1,
                fire_frame: 0,
                interrupt_frame: -1,
            },
            sub_action: false,
            repaths: false,
            runs: true,
        };
        t.players[1].objects[0].attack = Some(attack);
        t.players[1].attributes[213] = 0.5;
        t.players[1].generated[3] = 0.999;
    }

    #[test]
    fn a_keshik_makes_gold_while_it_fights_a_target_of_its_task_s_class() {
        let mut t = world();
        keshik(&mut t);
        give_master(&mut t, 4, 70, 0, Vec::new());
        put(&mut t, 2, 77, 4, 30.0, [6.5, 5.5]);
        run(&mut t, 1, 60);
        let given = 0.5_f32 * 0.01 * 0.5 + 0.999;
        assert_eq!(t.players[1].resources[3], given);
        assert_eq!(t.players[1].attributes[49], given);
        assert_eq!(t.players[1].attributes[169], given);
        assert_eq!(t.players[1].generated[3], 0.0);
        // in another state, nothing
        t.players[1].objects[0].attack.as_mut().unwrap().state = 6;
        run(&mut t, 1, 60);
        assert_eq!(t.players[1].generated[3], 0.0);
    }

    #[test]
    fn a_keshik_reads_its_target_as_its_update_left_it() {
        let mut t = world();
        keshik(&mut t);
        // the world holds a target of another class; the pending one, of class 0, counts
        give_master(&mut t, 4, 70, 6, Vec::new());
        put(&mut t, 2, 77, 4, 30.0, [6.5, 5.5]);
        let mut v = Victim {
            id: 77,
            hp: 12.0,
            class: 0,
            ..Victim::default()
        };
        let mut pending = Pending {
            victim: Some(&mut v),
            blasted: &mut [],
        };
        assert_eq!(
            t.abilities((60, 1, 0), 1, false, &mut pending),
            Ok(Vec::new())
        );
        assert_eq!(t.players[1].generated[3], 0.0);
        assert!(0.999 < t.players[1].resources[3]);
        // the world's alone: class 6, nothing
        t.players[1].generated[3] = 0.5;
        run(&mut t, 1, 60);
        assert_eq!(t.players[1].generated[3], 0.5);
    }

    /// A combat unit of master 9 at (5.5, 5.5) moving east, with a movement damage of range 2,
    /// 5 hit points and a timer of 3 s.
    fn trampler(t: &mut Turns) {
        let trample = Task {
            work_range: 2.0,
            work_val1: 5.0,
            work_val2: 3.0,
            ..task(MOVEMENT_DAMAGE)
        };
        give_master(t, 9, 70, 12, vec![trample]);
        put(t, 1, 10, 9, 100.0, [5.5, 5.5]);
        let mut mover = crate::hunt::tests::walker();
        mover.velocity = Some(crate::movement::Velocity {
            velocity: [1.0, 0.0, 0.0],
            acceleration: [0.0; 3],
        });
        t.players[1].objects[0].mover = Some(mover);
    }

    #[test]
    fn a_moving_unit_s_movement_damage_hits_the_enemies_about_it() {
        let mut t = world();
        trampler(&mut t);
        give_master(&mut t, 4, 70, 6, Vec::new());
        // an enemy in reach, one out of it, one of its own player, one of Gaia
        put(&mut t, 2, 20, 4, 30.0, [6.5, 5.5]);
        put(&mut t, 2, 21, 4, 30.0, [9.5, 5.5]);
        put(&mut t, 1, 30, 4, 30.0, [5.5, 6.5]);
        put(&mut t, 0, 40, 4, 30.0, [4.5, 5.5]);
        run(&mut t, 1, 10);
        let hp = |t: &Turns, id| t.object(id).unwrap().1.life.hp;
        assert_eq!(hp(&t, 20), 25.0);
        assert_eq!(hp(&t, 21), 30.0);
        assert_eq!(hp(&t, 30), 30.0);
        assert_eq!(hp(&t, 40), 30.0);
        assert_eq!(t.players[1].objects[0].abilities.movement_damage_timer, 3.0);
        // the timer runs down first: no hit for 3 s
        run(&mut t, 1, 10);
        assert_eq!(hp(&t, 20), 25.0);
        assert_eq!(t.players[1].objects[0].abilities.movement_damage_timer, 2.5);
        // standing, no hit even when due
        t.players[1].objects[0].abilities.movement_damage_timer = 0.0;
        t.players[1].objects[0].mover.as_mut().unwrap().velocity = None;
        run(&mut t, 1, 10);
        assert_eq!(hp(&t, 20), 25.0);
    }

    #[test]
    fn a_movement_damage_kill_leaves_0_hit_points_and_counts_the_kill() {
        let mut t = world();
        trampler(&mut t);
        give_master(&mut t, 4, 70, 6, Vec::new());
        put(&mut t, 2, 20, 4, 3.0, [6.5, 5.5]);
        t.players[2].objects[0].life.amount = 10.0;
        run(&mut t, 1, 10);
        let (_, u) = t.object(20).unwrap();
        assert_eq!((u.life.hp, u.life.amount), (0.0, 0.0));
        // FUN_1412005b0: player 1 a kill (20) of player 2 (302), player 2 a loss (154)
        assert_eq!(t.players[1].attributes[20], 1.0);
        assert_eq!(t.players[1].attributes[302], 1.0);
        assert_eq!(t.players[2].attributes[154], 1.0);
        assert_eq!(t.players[2].attributes[326], 1.0);
    }

    #[test]
    fn a_movement_damage_on_the_pending_target_hits_it_as_the_update_left_it() {
        let mut t = world();
        trampler(&mut t);
        give_master(&mut t, 4, 70, 6, Vec::new());
        put(&mut t, 2, 20, 4, 30.0, [6.5, 5.5]);
        let mut v = crate::turn::victim_of(
            &t.players[2].objects[0],
            t.masters[2].get(4).unwrap(),
            &t.players[2].attributes,
            2,
            None,
            0,
        );
        v.hp = 8.0;
        let mut pending = Pending {
            victim: Some(&mut v),
            blasted: &mut [],
        };
        assert_eq!(
            t.abilities((10, 1, 0), 1, false, &mut pending),
            Ok(Vec::new())
        );
        assert_eq!(v.hp, 3.0);
        // the world's copy waits for the write-back
        assert_eq!(t.object(20).unwrap().1.life.hp, 30.0);
    }

    /// The map's terrain: every tile of terrain 1 but (5, 6) of terrain 2, whose flag byte has
    /// 32 (deep water).
    fn waters(t: &mut Turns) {
        let mut tiles = vec![(1u8, 255u8, -1i16); 400];
        tiles[6 * 20 + 5] = (2, 255, -1);
        let kind = |flags| crate::terrain::TerrainKind {
            shown: true,
            flags,
            forest: false,
        };
        t.spatial.terrain = Some(crate::terrain::Terrain::new(
            20,
            20,
            tiles,
            vec![kind(0), kind(1), kind(0x21)],
            Vec::new(),
        ));
        let g = |id| crate::sprite::Graphic {
            id,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        };
        for id in [19598, 19599, 19600, 19601] {
            let _ = t.graphics.insert(id, g(id));
        }
    }

    #[test]
    fn a_longboat_takes_its_task_s_look_on_its_terrains_and_its_master_s_off_them() {
        let mut t = world();
        waters(&mut t);
        give_master(&mut t, 250, 70, 22, vec![longboat_look()]);
        put(&mut t, 1, 70, 250, 100.0, [5.5, 6.5]);
        let mut mover = crate::hunt::tests::walker();
        mover.moved_at = [5.5, 6.5, 0.0];
        t.players[1].objects[0].mover = Some(mover);
        run(&mut t, 1, 70);
        let u = &t.players[1].objects[0];
        assert_eq!(u.mover.as_ref().unwrap().factor, 0.35);
        assert_eq!(u.abilities.attack_rate, 1.0);
        assert_eq!(
            u.abilities.look,
            Some(Look {
                standing: Some(19599),
                walking: Some(19601),
                attacking: Some(19599),
                running: None,
                dying: Some(19598),
                undead: Some(19600),
            })
        );
        // where it last moved to decides, not its place
        t.players[1].objects[0].mover.as_mut().unwrap().moved_at = [5.5, 7.5, 0.0];
        run(&mut t, 1, 70);
        let u = &t.players[1].objects[0];
        assert_eq!(u.mover.as_ref().unwrap().factor, 1.0);
        assert_eq!((u.abilities.attack_rate, u.abilities.look), (1.0, None));
    }

    #[test]
    fn a_dead_object_that_leaves_issues_its_removal_rewards() {
        let mut t = world();
        let far = Task {
            work_range: 1.0,
            ..bounty()
        };
        give_master(&mut t, 5, 70, 6, vec![bounty(), far, bounty()]);
        put(&mut t, 1, 80, 5, 0.0, [5.5, 5.5]);
        t.players[1].attributes[7] = 2.0;
        assert_eq!(rewards(&mut t, 1, 80, false), Vec::new());
        let r = Reward {
            player: 1,
            attribute: 3,
            amount: 20,
        };
        assert_eq!(rewards(&mut t, 1, 80, true), vec![r, r]);
        // alive, it issues none
        t.players[1].objects[0].life.hp = 1.0;
        assert_eq!(rewards(&mut t, 1, 80, true), Vec::new());
    }

    #[test]
    fn an_object_of_another_type_has_no_abilities() {
        let mut t = world();
        give_master(&mut t, 7, 30, 6, vec![bounty()]);
        put(&mut t, 1, 90, 7, 0.0, [5.5, 5.5]);
        assert_eq!(rewards(&mut t, 1, 90, true), Vec::new());
    }

    /// `FUN_1416625b0`: an attribute below the player's count, the resources' from the slice's
    /// own (food, wood, stone and gold); none past it.
    #[test]
    fn a_player_s_attribute_reads_the_resources_first() {
        let p = Player {
            resources: [1.0, 2.0, 3.0, 4.0],
            attributes: vec![9.0; 6],
            ..Player::default()
        };
        assert_eq!(p.attribute(2), Some(3.0));
        assert_eq!(p.attribute(5), Some(9.0));
        assert_eq!(p.attribute(6), None);
    }
}
