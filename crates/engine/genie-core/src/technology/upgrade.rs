//! An object's upgrade to another master: slot 0x98 (`FUN_141201a60` for a combat object,
//! `FUN_1411c4660` for a building), down the classes' overrides (`FUN_14107cb70`,
//! `FUN_14106a530`, `FUN_14114cb80`) to the static object's `FUN_141186580`, as the upgrade of
//! its master runs it (`FUN_141657d10`, [`Turns::upgrade_objects`]); and the damage sprites'
//! check (`FUN_1411859a0`), which the static part runs around the hit points' change.

use crate::GenieError;
use crate::math::rng::{Stream, Well512};
use crate::objects::masters::{Master, Task, master_type};
use crate::objects::sprite::Graphic;
use crate::orders::Place;
use crate::world::{Turns, Unit};

/// A master's hit points this near 0 give no share (`ABS(x) <= 1.1920929e-07`).
const NO_HIT_POINTS: f32 = f32::EPSILON;
/// The whole of a damage share (`DAT_14330ebcc`, 100).
const WHOLE: f32 = crate::math::image::upgrade::WHOLE;
/// A sprite the data file does not have.
const NO_SPRITE: GenieError = GenieError::unsupported("a sprite the data file does not have");
/// The villager class (4), whose load an upgrade without its master kept goes to its player.
const VILLAGER: i16 = 4;
/// The food kinds a villager's load gives as food (15, 16 and 17).
const FOODS: [i16; 3] = [15, 16, 17];
/// The resource classes whose damage share is the share of their storage left from state 3 on
/// (`FUN_141193590`), and those whose share is it once dead (state above 2).
const RESOURCE_CLASSES: [i16; 11] = [5, 9, 10, 29, 31, 33, 38, 40, 41, 58, 61];
const RESOURCE_CLASSES_DEAD: [i16; 6] = [7, 8, 15, 32, 48, 63];
/// The class pairs whose change makes no new unit AI (`FUN_1415c6aa0`: the new class first).
const SAME_AI_CLASSES: [(i16, i16); 3] = [(12, 47), (18, 43), (43, 18)];

/// What a damage sprite apply mode does to the simulation (`FUN_1411859a0`'s switch on the
/// entry's `+0x9`).
const SPRITE_PLACED: u8 = 0;
const SPRITE_SCATTERED: u8 = 1;
const SPRITE_SHOWN: u8 = 2;

/// Slot 0xa8 (`FUN_141201670` for a combat object and a building; the base's `FUN_14106a520`
/// answers 1): whether an object of a master of type `kind` in state `state` takes an upgrade
/// (below state 4) rather than a master of its own.
#[must_use]
pub const fn upgradable(kind: u8, state: u8) -> bool {
    kind < master_type::COMBAT || state < 4
}

/// `FUN_141193590`: an object of `master` in `state` whose damage share is its storage's
/// (`+0x681` set, or a resource class: those of [`RESOURCE_CLASSES`] and
/// [`RESOURCE_CLASSES_DEAD`] in a state above 2).
fn storage_share(master: &Master, state: u8) -> bool {
    master.auto_gather
        || ((RESOURCE_CLASSES.contains(&master.class)
            || RESOURCE_CLASSES_DEAD.contains(&master.class))
            && 2 < state)
}

/// The damage sprites' check (`FUN_1411859a0`) of object `u` reading `master`, whose damage share
/// (`+0xd`) is `before`: a master without damage sprites leaves it. Else the share is that of its
/// storage left for a resource object ([`storage_share`]: the first storage entry of the kind it
/// holds, `100 - amount * 100 / storage`, CVTTSS2SI; 0 for none), or of its hit points, which
/// first stay within 0 and the master's (COMISS: a NaN stays; a master below 1 hit point leaves
/// the share). A changed share below 100 that moves the object to another damage sprite (the
/// last entry whose percent lies below the share, the old one's below the old share) removes the
/// object's attachments and shows the new entry by its apply mode: mode 0 places it on the
/// object (`FUN_14118a9d0`, else a decal), mode 1 at a random spot of its footprint (two draws of
/// the Object stream, `CheckDamageSprites`, then a decal), mode 2 as the object's sprite (slot
/// 0x68); then the old entry goes: a decal of mode 0 or 1, or for mode 2 the master's standing
/// sprite (slots 0x798 and 0x68) unless the new entry also shows one. The attachments and decals
/// are drawn, not simulated. Returns the share (`+0xd`), which the slice does not hold.
///
/// # Errors
///
/// A sprite the data file does not have.
pub(crate) fn damage_sprites(
    u: &mut Unit,
    master: &Master,
    before: u8,
    (graphics, object, sprite): (
        &std::collections::BTreeMap<i16, Graphic>,
        &mut Well512,
        &mut Well512,
    ),
) -> Result<u8, GenieError> {
    let entries = &master.damage_graphics;
    let Some((share, hp)) = share_of(u, master) else {
        return Ok(before);
    };
    u.life.hp = hp;
    if share == before || 100 <= share {
        return Ok(share);
    }
    let (mut now, mut was) = (None, None);
    for (i, e) in entries.iter().enumerate() {
        if e.percent < share {
            now = Some(i);
        }
        if e.percent < before {
            was = Some(i);
        }
    }
    if now == was {
        return Ok(share);
    }
    let shown = |i: Option<usize>| i.map(|i| entries[i]).filter(|e| 0 <= e.sprite);
    let graphic = |id: i16| graphics.get(&id).copied().ok_or(NO_SPRITE);
    if let Some(e) = shown(now) {
        match e.apply_mode {
            SPRITE_SCATTERED => {
                let _ = (object.rand15(), object.rand15());
            }
            SPRITE_SHOWN if !graphics.is_empty() => {
                if u.sprite.is_some() {
                    crate::combat::attack::set_sprite(&mut u.sprite, graphic(e.sprite)?, sprite);
                } else {
                    let _ = crate::objects::sprite::ActiveSprite::start(graphic(e.sprite)?, sprite);
                }
            }
            _ => {}
        }
    }
    if let Some(e) = shown(was)
        && e.apply_mode == SPRITE_SHOWN
        && shown(now).is_none_or(|n| n.apply_mode != SPRITE_SHOWN)
        && !graphics.is_empty()
    {
        let standing = graphic(master.standing_sprite)?;
        if u.sprite.is_some() {
            crate::combat::attack::set_sprite(&mut u.sprite, standing, sprite);
        } else {
            let _ = crate::objects::sprite::ActiveSprite::start(standing, sprite);
        }
    }
    let _ = SPRITE_PLACED;
    Ok(share)
}

/// The damage share [`damage_sprites`] finds for object `u` under `master`, with its hit points
/// after it (within 0 and the master's for a share of hit points); None where it leaves the
/// share (no damage sprites, or a master below 1 hit point).
fn share_of(u: &Unit, master: &Master) -> Option<(u8, f32)> {
    if master.damage_graphics.is_empty() {
        return None;
    }
    let mut hp = u.life.hp;
    if storage_share(master, u.life.state) {
        let held = master
            .storage
            .iter()
            .take(3)
            .find(|&&(kind, _, _)| kind == u.life.attribute_type);
        let share = held.map_or(0, |&(_, amount, _)| {
            crate::world::checkpoint::cvttss2si(WHOLE - u.life.amount * WHOLE / amount) as u8
        });
        return Some((share, hp));
    }
    if master.hit_points < 1 {
        return None;
    }
    let most = f32::from(master.hit_points);
    // COMISS hp, most; JBE: a NaN keeps; then COMISS 0, hp; JBE
    if most < hp {
        hp = most;
    } else if hp < 0.0 {
        hp = 0.0;
    }
    Some((
        crate::world::checkpoint::cvttss2si(WHOLE - hp * WHOLE / most) as u8,
        hp,
    ))
}

/// The damage share (`+0xd`) object `u` holds under `master`, which every change of its hit
/// points or master set through [`damage_sprites`] (the slice does not hold it); 0 where that
/// check would leave it.
fn damage_share_of(u: &Unit, master: &Master) -> u8 {
    share_of(u, master).map_or(0, |(s, _)| s)
}

/// `FUN_1415c6aa0` on player `p`'s masters `new` and `old` (by id): the unit's new class needs
/// another unit AI (`FUN_141215a90`) when the classes differ, but for the pairs of
/// [`SAME_AI_CLASSES`].
fn class_changes(new: Option<&Master>, old: Option<&Master>) -> bool {
    match (new, old) {
        (Some(n), Some(o)) => n.class != o.class && !SAME_AI_CLASSES.contains(&(n.class, o.class)),
        _ => false,
    }
}

impl Turns {
    /// Slot 0x98 of player `p`'s object at `place`, whose master `old` upgrades to `new`; with
    /// `keep` (the upgrade's modes −1 and 3, whose master then copies `new`) its master stays.
    /// In the game's order, outermost class first:
    ///
    /// - a building (`FUN_1411c4660`) showing `old`'s construction sprite shows `new`'s (slot
    ///   0x68); its snow decal swaps between a push and a pop of the Object stream (drawn);
    /// - a combat object (`FUN_141201a60`) takes the change of the missiles per volley
    ///   (`+0xa00`); its garrison decal swaps (drawn);
    /// - a combat object (`FUN_14107cb70`) showing `old`'s attack sprite shows `new`'s; slot
    ///   0x8a8 rebuilds what the object keeps of `new`'s tasks, which the slice reads from the
    ///   master;
    /// - an action object (`FUN_14106a530`): each action takes `new`'s task of its type, class
    ///   and target (`FUN_140a97970`, [`Turns::actions_take_master`]); without `keep` a top
    ///   action of a type `new` has no task for stops the object (slot 0x148);
    /// - a moving object (`FUN_14114cb80`) showing `old`'s walking or running sprite shows
    ///   `new`'s;
    /// - the static object (`FUN_141186580`): without `keep` a combat object leaves its class
    ///   counters ([`crate::world::count_class`] with −1) and a villager's load goes to its
    ///   player (slot 0xf8 with 1 and 1, the foods as food; the statistics are not
    ///   simulation); on the map in state 2 its sight goes and comes with `new`'s line of sight
    ///   (slots 0x1d8 and 0x1d0); its hit points take their share of `new`'s (`old`'s through
    ///   the damage sprites at full hit points first, [`damage_sprites`]); showing `old`'s
    ///   standing, second standing, dying or undead sprite it shows `new`'s; a combat object of
    ///   two masters with charges takes the change of the maximum (`+0x9e0`); in state 2 the
    ///   damage sprites again with `new`; without `keep` the master is `new` and the class
    ///   counters take it back (+1);
    /// - a combat object whose class changes takes a new unit AI ([`class_changes`],
    ///   `FUN_141215a90`); a building with an adjacent mode turns its facets (`FUN_1411ce890`,
    ///   drawn).
    ///
    /// Not simulation: the particles (`DAT_143e40318`), the AI player's list (`+0x7a8`,
    /// `FUN_141612dd0`, `FUN_14160c400`) and the statistics. An object of an overridden sprite
    /// (`+0x788`) would read the sprites of the master it names.
    ///
    /// # Errors
    ///
    /// A sprite the data file lacks, a load given to a sharing player with allies, and what the
    /// actions' change and the stop do not model.
    pub(crate) fn upgrade_object(
        &mut self,
        p: usize,
        place: Place,
        (old, new): (&Master, &Master),
        keep: bool,
    ) -> Result<(), GenieError> {
        let kind = old.kind;
        let id = self.object_at(place).id;
        if master_type::BUILDING <= kind {
            let (from, to) = (
                old.building.as_ref().map_or(-1, |b| b.construction_sprite),
                new.building.as_ref().map_or(-1, |b| b.construction_sprite),
            );
            self.show_if(place, from, to)?;
        }
        let class_changed = master_type::COMBAT <= kind
            && class_changes(
                self.masters.get(p).and_then(|m| m.get(new.id)),
                self.masters.get(p).and_then(|m| m.get(old.id)),
            );
        if master_type::COMBAT <= kind {
            let u = self.object_at_mut(place);
            u.firepower += new.volley_fire - old.volley_fire;
        }
        if master_type::RGE_COMBAT <= kind && 0 <= new.attack_sprite {
            self.show_if(place, old.attack_sprite, new.attack_sprite)?;
        }
        if master_type::ACTION <= kind {
            self.actions_take_master(place, old, new);
            if !keep
                && let Some(top) = self.object_at(place).top_action_kind()
                && !new.tasks.iter().any(|t| t.action_type == top)
                && let Place::Live(q, i) = place
            {
                self.object_stop(q, i)?;
            }
        }
        if master_type::MOVING <= kind {
            let shows = |s: i16| {
                self.object_at(place)
                    .sprite
                    .is_some_and(|a| 0 <= s && a.graphic.id == s)
            };
            if shows(old.move_sprite) {
                if 0 <= new.move_sprite {
                    self.show(place, new.move_sprite)?;
                }
            } else if shows(old.run_sprite) && 0 <= new.run_sprite {
                self.show(place, new.run_sprite)?;
            }
        }
        // the static object's part
        if !keep {
            if master_type::COMBAT <= kind {
                crate::world::count_class(&mut self.players[p].attributes, old, -1.0);
            }
            let u = self.object_at(place);
            if master_type::COMBAT <= kind
                && old.class == VILLAGER
                && 0.0 < u.life.amount
                && 0 <= u.life.attribute_type
            {
                let kind = if FOODS.contains(&u.life.attribute_type) {
                    0
                } else {
                    u.life.attribute_type
                };
                let amount = u.life.amount;
                let mut adds = Vec::new();
                crate::world::give_load(
                    (&self.spatial.relations, self.players[p].sharing),
                    p,
                    &mut adds,
                    kind,
                    amount,
                )?;
                for (k, v) in adds {
                    self.players[p].add_attribute(k, v)?;
                }
                let u = self.object_at_mut(place);
                u.life.attribute_type = -1;
                u.life.amount = 0.0;
            }
        }
        let u = self.object_at(place);
        if u.life.state == 2
            && u.life.inside.is_none()
            && let Some(placed) = self.spatial.get(id).copied()
            && let Some(sight) = self.spatial.sight.as_mut()
        {
            let now = sight.seer(id).map_or(old.line_of_sight, |s| s.los);
            let los = (now - old.line_of_sight) + new.line_of_sight;
            sight.sight_off(id, placed.position, placed.radius, false, false);
            let _ = sight.sight_on_los(
                id,
                placed.position,
                placed.radius,
                false,
                crate::world::checkpoint::cvttss2si(los),
                false,
            );
            if let Some(s) = sight.seer_mut(id) {
                s.los = los;
                if !keep {
                    s.master = new.id;
                }
            }
        }
        let u = self.object_at(place);
        let most = f32::from(old.hit_points);
        let share = if most.abs() <= NO_HIT_POINTS {
            0.0
        } else {
            u.life.hp / most
        };
        let before = damage_share_of(u, old);
        let state = u.life.state;
        self.object_at_mut(place).life.hp = most;
        let mut held = before;
        if state == 2 {
            held = self.damage_sprites_at(place, old, held)?;
        }
        let sprites = [
            (old.standing_sprite, new.standing_sprite),
            (
                old.standing_sprite_2.unwrap_or(-1),
                new.standing_sprite_2.unwrap_or(new.standing_sprite),
            ),
            (old.death_sprite, new.death_sprite),
            (old.undead_sprite, new.undead_sprite),
        ];
        let shown = self.object_at(place).sprite.map(|s| s.graphic.id);
        let next = sprites
            .iter()
            .find(|&&(from, _)| 0 <= from && shown == Some(from))
            .map(|&(_, to)| to);
        if master_type::COMBAT <= kind
            && master_type::COMBAT <= new.kind
            && 0 < old.charge_type
            && 0 < new.charge_type
        {
            let more = (new.max_charge - old.max_charge).abs();
            if 0.0 < more {
                self.object_at_mut(place).charge += more;
            }
        }
        self.object_at_mut(place).life.hp = f32::from(new.hit_points) * share;
        if let Some(s) = next.filter(|&s| 0 <= s) {
            self.show(place, s)?;
        }
        if state == 2 {
            let _ = self.damage_sprites_at(place, new, held)?;
        }
        if !keep {
            self.object_at_mut(place).life.master_id = new.id;
            if master_type::COMBAT <= kind {
                crate::world::count_class(&mut self.players[p].attributes, new, 1.0);
            }
        }
        if class_changed {
            self.new_unit_ai(p, place, new)?;
        }
        Ok(())
    }

    /// [`damage_sprites`] on the object at `place` with the world's graphics and streams.
    fn damage_sprites_at(
        &mut self,
        place: Place,
        master: &Master,
        before: u8,
    ) -> Result<u8, GenieError> {
        let graphics = std::mem::take(&mut self.graphics);
        let mut object = std::mem::take(self.streams.get(Stream::Object));
        let mut sprite = std::mem::take(self.streams.get(Stream::Sprite));
        let u = self.object_at_mut(place);
        let r = damage_sprites(u, master, before, (&graphics, &mut object, &mut sprite));
        self.graphics = graphics;
        *self.streams.get(Stream::Object) = object;
        *self.streams.get(Stream::Sprite) = sprite;
        r
    }

    /// Slot 0x68 on the object at `place` with sprite `to` when it shows `from` (both sprites).
    fn show_if(&mut self, place: Place, from: i16, to: i16) -> Result<(), GenieError> {
        let shows = self
            .object_at(place)
            .sprite
            .is_some_and(|s| 0 <= from && s.graphic.id == from);
        if shows && 0 <= to {
            self.show(place, to)?;
        }
        Ok(())
    }

    /// Slot 0x68 on the object at `place` with sprite `id`: a new instance unless it shows it
    /// already ([`crate::combat::attack::set_sprite`] on the Sprite stream). Without the graphics (no
    /// data file) the Sprite stream is not modelled.
    fn show(&mut self, place: Place, id: i16) -> Result<(), GenieError> {
        if self.graphics.is_empty() {
            return Ok(());
        }
        let g = self.graphics.get(&id).copied().ok_or(NO_SPRITE)?;
        let rng = self.streams.get(Stream::Sprite);
        let u = match place {
            Place::Live(q, i) => &mut self.players[q].objects[i],
            Place::Sleeping(q, i) => &mut self.players[q].sleeping[i],
        };
        crate::combat::attack::set_sprite(&mut u.sprite, g, rng);
        Ok(())
    }

    /// `FUN_141215a90` on the object at `place`, whose master became `new`: its unit AI goes and
    /// the factory's type for `new` makes another ([`crate::unit_ai::factory_type`],
    /// [`crate::unit_ai::UnitAi::constructed`] on the AI stream), none for a class without.
    fn new_unit_ai(&mut self, p: usize, place: Place, new: &Master) -> Result<(), GenieError> {
        let position = self.object_at(place).position;
        let made = self.new_object(p, new.id, new, position)?;
        let ai = crate::unit_ai::factory_type(
            new.class,
            new.id,
            new.volley_missile.unwrap_or(-1),
            new.combat_level,
        )
        .and_then(|t| crate::unit_ai::UnitAi::constructed(t, &made, self.streams.get(Stream::Ai)));
        self.object_at_mut(place).ai = ai;
        Ok(())
    }

    /// `FUN_140a97970` -> `FUN_140a78150` for each action of the object at `place` whose master
    /// `old` gives way to `new`: an action with a task takes the last of `new`'s tasks of the same
    /// action type, target class and target master (none: it keeps its task and changes nothing
    /// else); then its slot 0x50 with the tasks: an Attack's sprites follow `new`'s
    /// ([`crate::technology::effect::follow_master`]), its sub-actions alike (`FUN_140a979b0`).
    pub(crate) fn actions_take_master(&mut self, place: Place, old: &Master, new: &Master) {
        let find = |task: &Task| {
            new.tasks
                .iter()
                .rev()
                .find(|t| {
                    t.action_type == task.action_type
                        && t.object_group == task.object_group
                        && t.object_id == task.object_id
                })
                .copied()
        };
        let old_task = |id: i16| old.tasks.iter().find(|t| t.id == id).copied();
        let graphics = &self.graphics;
        let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
        let sprites = crate::technology::effect::MasterSprites {
            moving: g(new.move_sprite),
            attack: g(new.attack_sprite),
            standing: g(new.standing_sprite),
            idle_attack: g(new.idle_attack_sprite),
            second: g(new.attack_sprite2),
            combat: master_type::COMBAT <= new.kind,
        };
        let u = match place {
            Place::Live(q, i) => &mut self.players[q].objects[i],
            Place::Sleeping(q, i) => &mut self.players[q].sleeping[i],
        };
        if let Some(attack) = u.attack.as_mut() {
            crate::technology::effect::follow_master(&mut attack.arms, None, &sprites);
        }
        if let Some(hunt) = u.hunt.as_mut()
            && let Some(t) = old_task(hunt.task.id).as_ref().and_then(find)
        {
            hunt.task = crate::gathering::hunt::HuntTask::of(&t);
            let task = hunt.task.sprites;
            if let Some(attack) = hunt.attack.as_mut() {
                crate::technology::effect::follow_master(&mut attack.arms, Some(task), &sprites);
            }
        }
        if let Some(gather) = u.gather.as_mut()
            && let Some(t) = old_task(gather.task.id).as_ref().and_then(find)
        {
            gather.task = crate::gathering::hunt::HuntTask::of(&t);
        }
        if let Some(make) = u.make.as_mut()
            && let Some(t) = find(&make.task)
        {
            make.task = t;
        }
        if let Some(fly) = u.fly.as_mut()
            && let Some(t) = find(&fly.task)
        {
            fly.task = t;
        }
        if let Some(w) = u.wait_solidify.as_mut()
            && let Some(t) = find(&w.task)
        {
            w.task = t;
        }
        if let Some(r) = u.repair.as_mut()
            && let Some(t) = old_task(r.task.id).as_ref().and_then(find)
        {
            r.task = crate::construction::repair::RepairTask {
                id: t.id,
                work_value: t.work_val1,
            };
        }
        if let Some(b) = u.build.as_mut()
            && let Some(t) = old_task(b.task.id).as_ref().and_then(find)
        {
            b.task.id = t.id;
            b.task.range = t.work_range;
            b.task.work_value = t.work_val1;
            b.task.resource = t.attribute_type;
        }
    }

    /// The object at `place`.
    pub(crate) fn object_at(&self, place: Place) -> &Unit {
        match place {
            Place::Live(q, i) => &self.players[q].objects[i],
            Place::Sleeping(q, i) => &self.players[q].sleeping[i],
        }
    }

    /// The object at `place`, to change.
    pub(crate) fn object_at_mut(&mut self, place: Place) -> &mut Unit {
        match place {
            Place::Live(q, i) => &mut self.players[q].objects[i],
            Place::Sleeping(q, i) => &mut self.players[q].sleeping[i],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::masters::{BuildingMaster, DamageGraphic, Masters};

    /// An animated graphic of `frames` frames with a random first frame (flags 5).
    const fn graphic(id: i16) -> Graphic {
        Graphic {
            id,
            frames: 10,
            facets: 1,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 5,
        }
    }

    /// A building master `id` of `hp` hit points with three damage sprites of `mode`, its
    /// standing sprite `id + 1000`, its death spawn `dead`.
    fn building(id: i16, hp: i16, mode: u8, dead: i16) -> Master {
        Master {
            id,
            kind: master_type::BUILDING,
            class: 3,
            hit_points: hp,
            standing_sprite: id + 1000,
            death_spawns: [dead, -1],
            damage_graphics: [25, 50, 75]
                .map(|percent| DamageGraphic {
                    sprite: id + 2000 + i16::from(percent),
                    percent,
                    apply_mode: mode,
                })
                .to_vec(),
            damage_sprites: true,
            building: Some(BuildingMaster {
                construction_sprite: id + 3000,
                ..BuildingMaster::default()
            }),
            ..Master::default()
        }
    }

    /// A unit of master `master` in `state` with `hp`.
    fn object(id: i32, master: i16, state: u8, hp: f32) -> Unit {
        Unit {
            id,
            life: crate::objects::lifecycle::Lifecycle {
                master_id: master,
                state,
                hp,
                ..crate::objects::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        }
    }

    /// `FUN_1411859a0`: a master without damage sprites leaves the share and the hit points;
    /// the hit points stay within 0 and the master's (a NaN stays); a share that moves the object
    /// to another entry draws twice from the Object stream for an entry of mode 1, shows the
    /// entry's sprite for mode 2 and the standing sprite again when a mode 2 entry goes, and
    /// does nothing to the simulation for mode 0.
    #[test]
    fn the_damage_sprites_follow_the_share_of_the_hit_points() {
        let graphics: std::collections::BTreeMap<i16, Graphic> = [70, 1070, 2095, 2120, 2145]
            .map(|id| (id, graphic(id)))
            .into_iter()
            .collect();
        let (mut object, mut sprite) = (Well512::default(), Well512::default());
        let bare = Master {
            hit_points: 100,
            ..Master::default()
        };
        let mut u = object_of(150.0);
        let r = damage_sprites(&mut u, &bare, 7, (&graphics, &mut object, &mut sprite));
        assert_eq!((r, u.life.hp), (Ok(7), 150.0));
        let placed = building(70, 100, SPRITE_PLACED, -1);
        let r = damage_sprites(&mut u, &placed, 0, (&graphics, &mut object, &mut sprite));
        assert_eq!((r, u.life.hp), (Ok(0), 100.0));
        let mut low = object_of(-5.0);
        let r = damage_sprites(&mut low, &placed, 0, (&graphics, &mut object, &mut sprite));
        assert_eq!((r, low.life.hp), (Ok(100), 0.0));
        let mut nan = object_of(f32::NAN);
        let _ = damage_sprites(&mut nan, &placed, 0, (&graphics, &mut object, &mut sprite));
        assert!(nan.life.hp.is_nan());
        // mode 0: nothing drawn
        let before = object.clone();
        let mut hurt = object_of(40.0);
        assert_eq!(
            damage_sprites(&mut hurt, &placed, 0, (&graphics, &mut object, &mut sprite)),
            Ok(60)
        );
        assert_eq!(object, before);
        // mode 1: two draws of the Object stream
        let scattered = building(70, 100, SPRITE_SCATTERED, -1);
        let mut expected = object.clone();
        let _ = (expected.rand15(), expected.rand15());
        let mut hurt = object_of(40.0);
        assert_eq!(
            damage_sprites(
                &mut hurt,
                &scattered,
                0,
                (&graphics, &mut object, &mut sprite)
            ),
            Ok(60)
        );
        assert_eq!(object, expected);
        // the same entry (25 < 30 and 40, below 50): nothing more
        let mut hurt = object_of(60.0);
        assert_eq!(
            damage_sprites(
                &mut hurt,
                &scattered,
                30,
                (&graphics, &mut object, &mut sprite)
            ),
            Ok(40)
        );
        assert_eq!(object, expected);
        // mode 2: the entry's sprite, then the standing one when it goes
        let shown = building(70, 100, SPRITE_SHOWN, -1);
        let mut u = Unit {
            sprite: Some(crate::objects::sprite::ActiveSprite::start(
                graphic(1070),
                &mut sprite,
            )),
            ..object_of(40.0)
        };
        assert_eq!(
            damage_sprites(&mut u, &shown, 0, (&graphics, &mut object, &mut sprite)),
            Ok(60)
        );
        assert_eq!(u.sprite.map(|s| s.graphic.id), Some(2120));
        u.life.hp = 100.0;
        assert_eq!(
            damage_sprites(&mut u, &shown, 60, (&graphics, &mut object, &mut sprite)),
            Ok(0)
        );
        assert_eq!(u.sprite.map(|s| s.graphic.id), Some(1070));
    }

    fn object_of(hp: f32) -> Unit {
        object(1, 70, 2, hp)
    }

    /// `FUN_141193590`: a dead tree's share is that of its storage left (the entry of the kind
    /// it holds; none, 0), a living one's that of its hit points.
    #[test]
    fn a_resource_object_s_damage_share_is_its_storage_s() {
        let tree = Master {
            class: 15,
            hit_points: 20,
            storage: vec![(1, 100.0, 0)],
            damage_graphics: vec![DamageGraphic {
                sprite: -1,
                percent: 50,
                apply_mode: 0,
            }],
            ..Master::default()
        };
        let mut u = object(1, 1, 3, 20.0);
        (u.life.attribute_type, u.life.amount) = (1, 25.0);
        assert_eq!(share_of(&u, &tree), Some((75, 20.0)));
        u.life.attribute_type = 2;
        assert_eq!(share_of(&u, &tree), Some((0, 20.0)));
        u.life.state = 2;
        u.life.hp = 5.0;
        assert_eq!(share_of(&u, &tree), Some((75, 5.0)));
    }

    /// Slot 0xa8 and `FUN_1415c6aa0`: a combat object or building takes an upgrade below state
    /// 4, any other object always; a class change makes a new unit AI but for the pairs 12/47,
    /// 18/43 and 43/18 (the new class first).
    #[test]
    fn who_takes_an_upgrade_and_who_a_new_unit_ai() {
        assert!(upgradable(master_type::BUILDING, 3));
        assert!(!upgradable(master_type::BUILDING, 4));
        assert!(!upgradable(master_type::COMBAT, 7));
        assert!(upgradable(master_type::ACTION, 7));
        let m = |class| Master {
            class,
            ..Master::default()
        };
        assert!(!class_changes(Some(&m(3)), Some(&m(3))));
        assert!(class_changes(Some(&m(6)), Some(&m(12))));
        assert!(!class_changes(Some(&m(12)), Some(&m(47))));
        assert!(class_changes(Some(&m(47)), Some(&m(12))));
        assert!(!class_changes(Some(&m(18)), Some(&m(43))));
        assert!(!class_changes(Some(&m(43)), Some(&m(18))));
        assert!(!class_changes(None, Some(&m(3))));
    }

    /// The Feudal Age's upgrade of the house (70 to 464, mode −1, `FUN_141657d10`): a house in
    /// state 2 keeps its master and its share of the hit points (275 of 550: 450 of 900), and
    /// shows the new standing sprite; a dead one (state 4) and the rubble of its old death spawn
    /// (1403, whose copy differs from 464's 1435) take masters of their own, the dead one also
    /// its death spawn's; the player's master 70 then takes 464's values but its id and class.
    #[test]
    fn an_age_s_upgrade_keeps_a_house_s_share_of_its_hit_points() {
        let old = Master {
            death_spawns: [1403, -1],
            ..building(70, 550, SPRITE_PLACED, 1403)
        };
        let new = Master {
            class: 3,
            ..building(464, 900, SPRITE_PLACED, 1435)
        };
        let rubble = |id, copy| Master {
            id,
            copy_id: copy,
            kind: master_type::STATIC,
            ..Master::default()
        };
        let mut house = object(10, 70, 2, 275.0);
        let mut sprites = Well512::default();
        house.sprite = Some(crate::objects::sprite::ActiveSprite::start(
            graphic(1070),
            &mut sprites,
        ));
        let player = crate::world::Player {
            objects: vec![house, object(11, 70, 4, 0.0), object(12, 1403, 2, 1.0)],
            ..crate::world::Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player]);
        t.masters = vec![Masters::new([
            old,
            new,
            rubble(1403, 1403),
            rubble(1435, 1435),
        ])];
        t.graphics = [1070, 1464, 3070, 3464]
            .map(|id| (id, graphic(id)))
            .into_iter()
            .collect();
        t.upgrade_objects(0, 70, 464, -1, None).unwrap();
        let [house, dead, rubble] = &t.players[0].objects[..] else {
            panic!("three objects");
        };
        assert_eq!((house.life.master_id, house.life.hp), (70, 450.0));
        assert_eq!(house.sprite.map(|s| s.graphic.id), Some(1464));
        assert!(house.own_master.is_none());
        assert_eq!(dead.own_master.as_deref().map(|m| m.hit_points), Some(550));
        assert_eq!(dead.dead_master.as_deref().map(|m| m.id), Some(1403));
        assert_eq!(rubble.own_master.as_deref().map(|m| m.id), Some(1403));
        let master = t.masters[0].get(70).unwrap();
        assert_eq!(
            (master.id, master.hit_points, master.death_spawns[0]),
            (70, 900, 1435)
        );
    }

    /// An upgrade without its master kept (mode 0): the objects take the new master, a combat
    /// object leaves its old class's counters and joins the new one's, a villager's load of
    /// berries goes to its player as food, and the missiles per volley take the change.
    #[test]
    fn an_upgrade_to_another_master_moves_the_class_counters_and_the_load() {
        let old = Master {
            id: 83,
            kind: master_type::COMBAT,
            class: VILLAGER,
            hit_points: 25,
            volley_fire: 1.0,
            ..Master::default()
        };
        let new = Master {
            id: 6,
            kind: master_type::COMBAT,
            class: 6,
            hit_points: 50,
            volley_fire: 3.0,
            ..Master::default()
        };
        let mut v = object(10, 83, 2, 25.0);
        (v.life.attribute_type, v.life.amount) = (17, 10.0);
        v.firepower = 1.0;
        let player = crate::world::Player {
            objects: vec![v],
            attributes: vec![0.0; 300],
            ..crate::world::Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player]);
        t.masters = vec![Masters::new([old, new])];
        t.players[0].attributes[37] = 1.0;
        t.upgrade_objects(0, 83, 6, 0, None).unwrap();
        let v = &t.players[0].objects[0];
        assert_eq!((v.life.master_id, v.life.hp, v.firepower), (6, 50.0, 3.0));
        assert_eq!((v.life.attribute_type, v.life.amount), (-1, 0.0));
        let a = &t.players[0].attributes;
        assert_eq!((a[37], a[40]), (0.0, 1.0));
        assert_eq!(t.players[0].resources[0], 10.0);
        assert_eq!(t.masters[0].get(83).map(|m| m.class), Some(VILLAGER));
    }
}
