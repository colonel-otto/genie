//! A hit's victim: what the hit reads of it, the objects it may reach, the hit written back,
//! the kill and its counts.

use crate::GenieError;
use crate::combat::damage::{Kill, Razed, Victim};
use crate::map::spatial::Spatial;
use crate::objects::masters::Master;
use crate::objects::unit::Unit;
use crate::unit_ai::Notify;
use crate::unit_ai::task_finder::Diplomacy;
use crate::unit_ai::task_finder::world::{SideWorld, TaskWorld};
use crate::world::context::Context;
use crate::world::others::OthersRef;
use crate::world::{BUILDING_KIND, World};

impl World {
    /// What a hit of an attacker of player `attacker` reads of the object `id` ([`Victim`]): an
    /// object the slice holds, live or asleep (the world's table holds both, and a blast walks
    /// the tiles' lists of both), with its relations to the attacker's player and whether its
    /// place is a guess.
    pub(crate) fn victim(&self, id: i32, attacker: usize) -> Option<Victim> {
        let (p, u) = self.players.iter().enumerate().find_map(|(p, player)| {
            player
                .objects
                .by_id(id)
                .or_else(|| player.sleeping.by_id(id))
                .map(|u| (p, u))
        })?;
        let master = u.master(&self.masters, p).ok()?;
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        Some(victim_in(
            u,
            (p, master, &self.players[p].attributes),
            (attacker, &world),
            &self.spatial,
            self.mode_680,
        ))
    }

    /// The object `id` as the unit AI of a unit of player `viewer` finds it in the world's object
    /// table (`FUN_1415417a0`, [`crate::unit_ai::Seen`]), `None` when no list holds it. An object
    /// the slice holds as a place alone cannot be read. Without the players' sight its tile
    /// counts as unseen.
    pub(crate) fn seen(
        &self,
        id: i32,
        viewer: usize,
    ) -> Result<Option<crate::unit_ai::Seen>, GenieError> {
        for (p, player) in self.players.iter().enumerate() {
            if let Some(u) = player
                .objects
                .by_id(id)
                .or_else(|| player.sleeping.by_id(id))
            {
                let master = u.master(&self.masters, p)?;
                let [x, y, _] = u.position();
                return Ok(Some(crate::unit_ai::Seen {
                    state: u.life.state,
                    kind: master.kind,
                    owner: p,
                    class: master.class,
                    position: u.position(),
                    radius: master.radius,
                    visible: self.spatial.visible(viewer, x as i32, y as i32),
                    flags: master.object_flags,
                    sight: self
                        .spatial
                        .tile_sight(
                            viewer,
                            crate::math::conversions::cvttss2si(x),
                            crate::math::conversions::cvttss2si(y),
                        )
                        .unwrap_or(0),
                }));
            }
        }
        match self.spatial.get(id) {
            Some(_) => Err(GenieError::unsupported(
                "a unit AI's target the slice cannot read",
            )),
            None => Ok(None),
        }
    }

    /// The objects besides `unit` (player `p`) and its `target` that its hit may reach: for a
    /// master with an area effect, the objects on the tiles of the blast's rectangle about the
    /// unit, a tile wider each way (the unit may step in its update), each with its place in its
    /// tile's list as the unit's update starts (an object the slice cannot read stands in with
    /// its place alone, as a guess); then the main buildings (`+0xbb8`) of the target and of the
    /// annexes among them, which the hits on those annexes go to.
    pub(crate) fn blasted(&self, unit: &Unit, p: usize, target: i32) -> Vec<Victim> {
        let reach = unit
            .master(&self.masters, p)
            .ok()
            .map(|m| m.area_effect)
            .filter(|&r| 0.0 < r)
            .map(|r| r + 6.0 + 2.0);
        let mut near: Vec<Victim> = reach.map_or_else(Vec::new, |reach| {
            let [x, y, _] = unit.position();
            self.spatial
                .objects_within(x - reach, y - reach, x + reach, y + reach)
                .filter(|&(id, _)| id != unit.id && id != target)
                .map(|(id, o)| {
                    self.victim(id, p).unwrap_or_else(|| Victim {
                        id,
                        position: o.position,
                        uncertain: true,
                        tile_place: self.spatial.tile_place(id),
                        ..Victim::default()
                    })
                })
                .collect()
        });
        let mains: Vec<i32> = std::iter::once(target)
            .chain(near.iter().map(|v| v.id))
            .filter_map(|id| self.spatial.annex_of(id))
            .collect();
        for m in mains {
            if m != unit.id
                && m != target
                && !near.iter().any(|v| v.id == m)
                && let Some(v) = self.victim(m, p)
            {
                near.push(v);
            }
        }
        near
    }
}

/// What a hit's write-back ([`write_back`]) leaves for the world ([`World::kill_left`], at the
/// hit through [`file_kill`]), in the order the hits made it: the object's kill and its razings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KillLeft {
    /// The object.
    pub(crate) id: i32,
    /// A hit killed it ([`World::killed`]).
    pub(crate) kill: Option<Kill>,
    /// What each hit that left it, a building, below 1 hit point counted.
    pub(crate) razed: Vec<Razed>,
}

impl KillLeft {
    /// The updating unit `id`, killed by its target's retaliation (`kill`).
    pub(crate) const fn own(id: i32, kill: Kill) -> Self {
        Self {
            id,
            kill: Some(kill),
            razed: Vec::new(),
        }
    }
}

/// The object `id` a hit of the updating unit can read ([`victim`]) with its owner and master:
/// one of any list, live or asleep, of a master its owner has.
pub(crate) fn readable<'o>(c: &'o Context<'_>, id: i32) -> Option<(usize, &'o Unit, &'o Master)> {
    let (p, u, _) = c.w.locate(id)?;
    let master = u.master(&c.w.masters, p).ok()?;
    Some((p, u, master))
}

/// What a hit of the updating unit reads of object `id` ([`World::victim`] from the update):
/// an object [`readable`] by the hit.
pub(crate) fn victim(c: &Context<'_>, id: i32) -> Option<Victim> {
    let (p, u, master) = readable(c, id)?;
    let view = OthersRef::of(&c.w.players, c.player);
    let attributes = if p == c.player {
        &c.w.players[c.player].attributes[..]
    } else {
        &view.players().player(p)?.attributes[..]
    };
    let sides = SideWorld {
        player: c.player,
        relations: &c.w.players[c.player].relations,
        foreign: view.players(),
        drop_sites: &c.w.drop_sites,
    };
    Some(victim_in(
        u,
        (p, master, attributes),
        (c.player, &sides),
        &c.w.spatial,
        c.w.mode_680,
    ))
}

/// The hit's changes on its victim `v`, written back to the world where the hit ran. An object asleep (a tree a blast reached) takes its hit points and what it
/// holds; below 1 hit point (COMISS/JBE: a NaN sleeps on) the hit wakes it (`FUN_14119deb0`
/// with 0, [`World::wake_sleeper`]) to take the rest. A live one takes its hit points, what it holds,
/// its sleep flag, its decay, its powerups and charge, a struck building's
/// `damaged_lately_timer`, the damage notifies (500) to its unit AI, and a struck building's
/// annexes take its hit points (`+0xbc0`..). Returns its kill and razings for the caller
/// ([`KillLeft`]); `v` keeps neither them nor its notifies, so another write-back repeats none.
///
/// # Errors
///
/// A notify its unit AI's queue does not take.
pub(crate) fn write_back(
    c: &mut Context<'_>,
    v: &mut Victim,
) -> Result<Option<KillLeft>, GenieError> {
    let damaged = std::mem::take(&mut v.damaged);
    let left = KillLeft {
        id: v.id,
        kill: v.killed.take(),
        razed: v
            .building
            .as_mut()
            .map(|b| std::mem::take(&mut b.razed))
            .unwrap_or_default(),
    };
    let Some((_, _, sleeps)) = c.w.locate(v.id) else {
        return Ok(None);
    };
    if sleeps {
        let (_, u) = c.w.object_mut(v.id).expect("located");
        u.life.hp = v.hp;
        u.life.amount = v.amount;
        if crate::math::fmath::at_most_or_unordered(1.0, v.hp) {
            return Ok(None);
        }
        let _ = c.w.wake_sleeper(v.id);
    }
    let (_, u) = c.w.object_mut(v.id).expect("located");
    u.life.hp = v.hp;
    u.life.amount = v.amount;
    u.life.goto_sleep = v.goto_sleep;
    // a hunt that stopped its decay gave it a master of its own (FUN_140ac44b0)
    if let Some(m) = u.life.master.as_mut() {
        m.decay = v.decay;
    }
    if let Some(a) = v.armored.as_ref() {
        u.hp_powerups = a.hp_powerups;
    }
    // a shield spends its charge
    u.charge = v.source.charge;
    let struck = v.building.as_ref().filter(|b| b.struck);
    if let Some(b) = struck {
        u.damaged_lately = b.damaged_lately;
    }
    // the damage notify (500) to its unit AI
    if let Some(ai) = u.ai.as_mut() {
        for &(attacker, p) in &damaged {
            let n = Notify {
                caller: attacker,
                recipient: v.id,
                kind: crate::unit_ai::DAMAGED,
                p,
            };
            ai.post(n, u.life.state)?;
        }
    }
    for &a in struck.map_or(&[][..], |b| &b.annexes) {
        if let Some((_, _, false)) = c.w.locate(a) {
            let (_, x) = c.w.object_mut(a).expect("located");
            x.life.hp = v.hp;
        }
    }
    Ok((left.kill.is_some() || !left.razed.is_empty()).then_some(left))
}

/// A hit's kill and razings where the hit runs (the damage slot `FUN_1412108e0` kills and counts
/// at once: `FUN_141082e80`, slot 0x8b0): another object's reach the world now
/// ([`World::kill_left`]); the updating object's own, its slot holding its stand-in, follow its
/// step ([`Context::kills`]).
///
/// # Errors
///
/// What the kill does not model, or an attribute a player lacks.
pub(crate) fn file_kill(c: &mut Context<'_>, k: KillLeft) -> Result<(), GenieError> {
    if k.id == c.me {
        c.kills.push(k);
        Ok(())
    } else {
        c.w.kill_left(&k)
    }
}

/// A strike's write-back where it ran ([`write_back`]), each object's kill and razings right
/// after it ([`file_kill`]): the blast's objects (`blasted`, which go back to the update's) in
/// their order, then its target `v`.
///
/// # Errors
///
/// A notify a unit AI's queue does not take, or what a kill does not model.
pub(crate) fn write_struck(
    c: &mut Context<'_>,
    mut blasted: Vec<Victim>,
    v: &mut Victim,
) -> Result<(), GenieError> {
    for b in &mut blasted {
        if let Some(k) = write_back(c, b)? {
            file_kill(c, k)?;
        }
    }
    c.blasted = blasted;
    if let Some(k) = write_back(c, v)? {
        file_kill(c, k)?;
    }
    Ok(())
}

/// A strike's write-back where it ran ([`write_struck`] on the update's `blasted`, then its
/// target `v`); what they leave joins the update's kills in that order.
///
/// # Errors
///
/// A notify a unit AI's queue does not take.
pub(crate) fn write_strike_back(c: &mut Context<'_>, v: &mut Victim) -> Result<(), GenieError> {
    let blasted = std::mem::take(&mut c.blasted);
    write_struck(c, blasted, v)
}

impl World {
    /// What a hit's write-back left after the update ([`KillLeft`]) on an object the world
    /// still holds live: its kill ([`World::killed`]), then each razing
    /// ([`crate::combat::damage::Razed`]: the history event of the attacker's player, the
    /// attribute adds in their order).
    ///
    /// # Errors
    ///
    /// What the kill does not model, or an attribute a player lacks.
    pub(crate) fn kill_left(&mut self, k: &KillLeft) -> Result<(), GenieError> {
        let Some(crate::orders::Place::Live(q, j)) = self.place(k.id) else {
            return Ok(());
        };
        if let Some(kill) = k.kill {
            self.killed(q, j, kill)?;
        }
        for r in &k.razed {
            if let Some(kind) = r.event {
                let now = self.world_time;
                self.players[r.attacker_player].history.add_event(
                    kind,
                    r.owner as f32,
                    0.0,
                    0,
                    now,
                );
            }
            for &(p, attribute, amount) in &r.adds {
                self.players[p].add_attribute(attribute, amount)?;
            }
        }
        Ok(())
    }

    /// A combat object's death by a hit (`FUN_141082e80`, `FUN_1412108e0`): its owner's computer
    /// AI is told (slot 0x2f0, not modelled); the killed handling (slot 0x8b0, `FUN_141205880` ->
    /// `FUN_1412005b0`): unless its class is one of the non-military ones the attacker's player
    /// counts a kill (attribute 20) and its owner a loss (154); the attacker's player counts it
    /// against the owner (300 + the owner) and the owner against the attacker (325 + the
    /// attacker's player); both count its paid cost (170 and 152, and 400 + the owner). A killed
    /// villager first counts for its owner (`FUN_141082e80`): an attacker of class 9 or 10 (an
    /// animal) in attribute 227, and by the attacker's player type a human's in 229, Gaia's in
    /// 226, a computer's in 228. Gaia takes a player's livestock (slot 0x70): its place and sight, its Artifact's
    /// wait, its unit AI's notifies (cleared, then 0x20b), its storage (the old owner loses flags 2,
    /// 16 and 32, Gaia gains 1, 2 and 32) and its list (the old owner's last object takes its
    /// place). Not held: the killer (`+0x690`, `+0x694`), `+0x954`, the statistics of the local
    /// player and the particles (`FUN_141158bb0`).
    pub(crate) fn killed(&mut self, q: usize, j: usize, kill: Kill) -> Result<(), GenieError> {
        if self.players[q].computer {
            return Err(GenieError::unsupported(
                "a computer player's killed object (its AI's slot 0x2f0)",
            ));
        }
        let u = &self.players[q].objects[j];
        let master = u.master(&self.masters, q)?.clone();
        let ap = kill.attacker_player;
        if master.class == 4 {
            // the attacker's class as the hit saw it: the attacker may be out for its update
            if matches!(kill.attacker_class, 9 | 10) {
                self.players[q].add_attribute(227, 1.0)?;
            }
            let by_kind = match self.players[ap].kind {
                1 => Some(229),
                2 => Some(226),
                3 => Some(228),
                _ => None,
            };
            if let Some(kind) = by_kind {
                self.players[q].add_attribute(kind, 1.0)?;
            }
        }
        self.count_kill_of(q, &master, ap)?;
        if kill.to_gaia {
            self.give_gaia(q, j)?;
        }
        Ok(())
    }

    /// `FUN_1412005b0` on the object `id` killed for player `killer` (a movement damage's,
    /// [`crate::combat::abilities`]): the counts of [`World::killed`] by its master (its own one, else
    /// its player's).
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, and an attribute whose change the slice does not
    /// model.
    pub(crate) fn count_kill(&mut self, id: i32, killer: usize) -> Result<(), GenieError> {
        let (q, u) = self
            .object(id)
            .ok_or(crate::construction::build::NOT_HELD)?;
        let master = u.master(&self.masters, q)?.clone();
        self.count_kill_of(q, &master, killer)
    }

    /// `FUN_1412005b0`'s counts of a kill of an object of player `q` and `master` for player `ap`:
    /// unless its class is one of the non-military ones (`FUN_141188b20`) `ap` counts a kill
    /// (attribute 20) and `q` a loss (154); `ap` counts it against `q` (300 + `q`) and `q`
    /// against `ap` (325 + `ap`); both count its paid cost (170 and 152, and 400 + `q`).
    pub(crate) fn count_kill_of(
        &mut self,
        q: usize,
        master: &Master,
        ap: usize,
    ) -> Result<(), GenieError> {
        const PEACEFUL: [i16; 26] = [
            1, 3, 5, 7, 8, 9, 10, 11, 14, 15, 16, 27, 29, 30, 31, 32, 33, 34, 38, 39, 40, 48, 52,
            58, 60, 63,
        ];
        if !PEACEFUL.contains(&master.class) {
            self.players[ap].add_attribute(20, 1.0)?;
            self.players[q].add_attribute(154, 1.0)?;
        }
        self.players[ap].add_attribute(q as i16 + 300, 1.0)?;
        self.players[q].add_attribute(ap as i16 + 325, 1.0)?;
        let cost = paid_cost(master);
        self.players[ap].add_attribute(170, cost)?;
        self.players[q].add_attribute(152, cost)?;
        self.players[ap].add_attribute(q as i16 + 400, cost)?;
        Ok(())
    }
}

/// What a hit of an attacker of player `attacker` reads of the object `u` of player `p`, of
/// `master`, its player's `attributes` ([`victim_of`]): its relations to the attacker's player
/// both ways (`dip`, [`Diplomacy::allied`] with allies), whether its place is a guess (the world
/// `spatial` holds it as unknown, or not at all), its place in its tile's list and a building's
/// annexes (the world's links).
pub(crate) fn victim_in(
    u: &Unit,
    (p, master, attributes): (usize, &Master, &[f32]),
    (attacker, dip): (usize, &dyn Diplomacy),
    spatial: &Spatial,
    mode_680: i32,
) -> Victim {
    let mut v = victim_of(u, master, attributes, p, spatial.terrain.as_ref(), mode_680);
    v.allied = [dip.allied(attacker, p, true), dip.allied(p, attacker, true)];
    v.uncertain = spatial.get(u.id).is_none_or(|o| o.unknown.is_some());
    v.tile_place = spatial.tile_place(u.id);
    if let Some(b) = v.building.as_mut() {
        b.annexes = spatial.annexes_of(u.id);
    }
    v
}

/// What a hit reads of the object `u` of player `p` ([`Victim`]): its master's values and its
/// own, its player's elevation attributes, the terrain defense of its tile (`terrain`, the map's;
/// 1 for an object inside another, which has no tile) and the game's setting `mode_680`. Its
/// relations to an attacker (`allied`) and whether its place is a guess (`uncertain`) are the
/// caller's. The slice holds neither its unit AI's alert mode and attackers nor its facing.
pub(crate) fn victim_of(
    u: &Unit,
    master: &Master,
    attributes: &[f32],
    p: usize,
    terrain: Option<&crate::map::terrain::Terrain>,
    mode_680: i32,
) -> Victim {
    let at = |i: usize| attributes.get(i).copied().unwrap_or(0.0);
    let kind = master.kind;
    let attack_elevation = if 212 < attributes.len() {
        [attributes[211], attributes[212]]
    } else {
        [0.0; 2]
    };
    let defense = if master.terrain_bonus < 0 || u.life.inside.is_some() {
        Some(1.0)
    } else {
        terrain.and_then(|t| {
            t.defense(
                master.terrain_bonus,
                u.position()[0] as i32,
                u.position()[1] as i32,
            )
        })
    };
    let tasks = |kind: i16| master.tasks.iter().any(|t| t.action_type == kind);
    Victim {
        id: u.id,
        // the caller's, which holds the tile lists
        tile_place: 0,
        hp: u.life.hp,
        amount: u.life.amount,
        state: u.life.state,
        position: u.position(),
        kind,
        class: master.class,
        master_id: master.id,
        radius: master.radius,
        height: master.height,
        storage: master.storage.first().map_or(0.0, |s| s.1),
        capacity: master.capacity,
        workers: u.workers,
        blast_defense: master.blast_defense,
        fog_visibility: master.fog_visibility,
        goto_sleep: u.life.goto_sleep,
        decay: u.life.master.map_or(0.0, |m| m.decay),
        player: p,
        elevation: [at(272), at(273)],
        attack_elevation,
        allied: [false; 2],
        uncertain: false,
        inside: u.life.inside.is_some(),
        source: crate::combat::damage::Attacker {
            id: u.id,
            kind,
            class: master.class,
            master_id: master.id,
            ability: master.combat_ability,
            missile: master.missile,
            weapon_range: master.weapon_range,
            area_effect: master.area_effect,
            blast_level: master.blast_level,
            area_damage: master.area_damage,
            charge_type: master.charge_type,
            max_charge: master.max_charge,
            charge_target: master.charge_target,
            special_attack: master.special_attack,
            charge: u.charge,
            charge_hit: u.charge_hit,
            attack_powerup: u.attack_powerup,
            mode_680,
            player: p,
            kill_bonus_tasks: tasks(0x9a),
            fired_from: u.missile.as_ref().map_or(-1, |m| m.fired_from),
        },
        weapons: master.weapons.clone(),
        master_hp: master.hit_points,
        bonus_tasks: master
            .tasks
            .iter()
            .filter(|t| t.action_type == crate::combat::damage::BONUS_DAMAGE)
            .copied()
            .collect(),
        // a combat object's slot 0xf0 is FUN_141082530 (types 50 and up)
        armored: (50 <= kind).then(|| crate::combat::damage::Armored {
            armors: master.armors.clone(),
            base_armor: master.base_armor,
            terrain_bonus: master.terrain_bonus,
            resistance: master.resistance,
            ability: master.combat_ability,
            reflection: master.reflection,
            invulnerability: master.invulnerability,
            special_attack: master.special_attack,
            friendly_fire: master.friendly_fire,
            special_graphic: master.special_graphic,
            terrain: defense,
            armor_powerups: u.armor_powerups,
            armor_removed: u.armor_removed,
            hp_powerups: u.hp_powerups,
            kill_tasks: master
                .tasks
                .iter()
                .any(|t| t.action_type == 0xa1 && 1e-7 < t.work_val1),
            busy: u.top_action_kind().is_some() || u.unmodelled_action.is_some(),
            unit_ai: u.ai.is_some(),
            alert_idle: None,
            facing: None,
        }),
        damaged: Vec::new(),
        killed: None,
        // a building's slot 0xe8 is FUN_1411cea90
        building: (kind == BUILDING_KIND).then(|| crate::combat::damage::Building {
            main: u.annex_of,
            // the caller's, which holds the world's annex links
            annexes: Vec::new(),
            // +0xb90 is not saved and not held: a blast resets it on every building of its tiles
            // before it reads it (FUN_14107f690's first pass), the only other one it reads being
            // the main building of an annex it reaches, which lies within the annex offset of
            // that annex, inside the six tiles the pass reaches past the radius; a trample reads
            // its target's or its main building's after its own blast
            blast_taken: 0.0,
            trebuchet: is_trebuchet(master),
            cost: paid_cost(master),
            damaged_lately: u.damaged_lately,
            struck: false,
            relics: u.relics,
            razed: Vec::new(),
        }),
    }
}

/// `FUN_140fde380`: `master` is of type 70 and up, of class 51 or 54, with a weapon (`+0x810`
/// above 0): a trebuchet, packed or not, whose kill counts as a unit's.
pub(crate) const fn is_trebuchet(master: &Master) -> bool {
    70 <= master.kind && matches!(master.class, 51 | 54) && !master.weapons.is_empty()
}

/// `master`'s paid cost (`+0x880`: each entry with a resource and the paid flag), summed as
/// floats in its order, as the kill counts read it (`FUN_1412005b0`, `FUN_1411cea90`).
pub(crate) fn paid_cost(master: &Master) -> f32 {
    let mut cost = 0.0f32;
    for &(resource, amount, paid) in &master.cost {
        if resource >= 0 && paid {
            cost += f32::from(amount);
        }
    }
    cost
}
