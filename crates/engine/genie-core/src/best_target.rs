//! `UnitAIModule::bestUnitToAttack` (158041 `0x140c44680`) and what it keeps: the attack
//! categories (`FUN_140c4c8e0`) by which a unit AI's order history weighs its choice.

/// `FUN_140c4c8e0`: the attack category of an object of master `master` and class `class`, by
/// which `bestUnitToAttack` weighs and narrows its choice; −1 for a class it does not rank.
#[must_use]
pub const fn category(master: i16, class: i16) -> i32 {
    match class {
        0 | 23 | 26 | 36 | 44 => {
            if matches!(master, 6 | 7) {
                4
            } else {
                3
            }
        }
        2 | 21 => 11,
        3 | 27 | 39 | 49 | 51 | 52 => 0,
        4 | 19 | 58 | 59 => 7,
        6 | 12 | 24 | 35 | 45 | 47 | 50 | 56 | 57 => {
            if matches!(master, 1744 | 1746) {
                5
            } else {
                1
            }
        }
        13 | 54 | 55 => {
            if matches!(master, 35 | 422 | 548 | 1258) {
                5
            } else {
                6
            }
        }
        17 | 18 | 43 => 8,
        20 | 62 => 10,
        22 | 53 => 9,
        28 | 46 => 2,
        _ => -1,
    }
}

/// The statistics' table holds at most this many objects (`FUN_140c66590`: `+0x1668` below
/// 0x80).
pub const STATISTICS: usize = 128;
/// A kill time's stand-in when the unit does no damage to an object it may still work on
/// (`DAT_14330ebb8`), and the current target's bonus.
const NO_DAMAGE_KILL: f32 = crate::image::best_target::FIVE;
/// The kill time of an object the unit neither damages nor works on (`DAT_14330ebf4`).
const NEVER: f32 = crate::image::best_target::NEVER;
/// Damage at most this does not count (`DAT_142eb0c60`), nor a span below it.
const EPSILON: f32 = crate::image::best_target::EPSILON;
/// The scores' scale (`DAT_142eb0f4c`), a distance score's top (`DAT_142efe418`) and the
/// targeting factor's per cent (`DAT_142eb99cc`).
const SCORE: f32 = crate::image::best_target::SCORE;
const DISTANCE_SCORE: f32 = crate::image::best_target::DISTANCE_SCORE;
const PER_CENT: f32 = crate::image::best_target::PER_CENT;
/// The rank's step per tier (`DAT_142eea788`).
const TIER: f32 = crate::image::best_target::TIER;
/// The classes whose objects stand fixed for the rank (`FUN_140c66590`'s mask
/// `0x12000000000008`: 3, 49, 52).
const FIXED_CLASSES: u64 = 0x0012_0000_0000_0008;

/// An object in the statistics (0x2c bytes at `DAT_143e74538`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Entry {
    /// `+0`: its id.
    pub id: i32,
    /// `+4`: it moves (its slot 0x208 above 0).
    pub moving: bool,
    /// `+5`: its class is one of [`FIXED_CLASSES`].
    pub fixed: bool,
    /// `+8`: the time the unit takes to kill it (its hit points over the unit's damage to it,
    /// over the unit's reload time).
    pub kill_time: f32,
    /// `+0xc`: the footprints' gap.
    pub distance: f32,
    /// `+0x10`: its attack category ([`category`]).
    pub category: i32,
    /// `+0x14`: the kill-time score.
    pub kill_score: f32,
    /// `+0x18`: the distance score.
    pub distance_score: f32,
    /// `+0x1c`: the order history's weight of its category.
    pub weight: f32,
    /// `+0x20`: the current target's bonus (5, else 0).
    pub bonus: f32,
    /// `+0x24`: the sum of the four.
    pub total: f32,
    /// `+0x28`: the rank the sort orders by ([`Statistics::rank`]).
    pub rank: f32,
}

/// What `FUN_140c66590` reads of the unit and of an object it adds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// The object.
    pub id: i32,
    /// Its footprints' gap to the unit (`FUN_14118f680`).
    pub distance: f32,
    /// Its attack category (the caller's `FUN_140c4c8e0`, counted when not −1).
    pub category: i32,
    /// Its master's class.
    pub class: i16,
    /// Its top speed (slot 0x208).
    pub speed: f32,
    /// It is armed: the sum of its attacks above 0 (slot 0x228, `FUN_141084070`), or its unit AI
    /// could work on the unit (its slot 0x1b8 with 1: a priest's conversion test).
    pub armed: bool,
    /// The unit's weapon range against it (slot 0x238).
    pub reach: f32,
    /// The unit's damage per hit to it (its slot 0x220, `FUN_141083f20`: the unit's elevation
    /// factor and its armor against the unit's attacks).
    pub damage: f32,
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// The unit's reload time (slot 0x218).
    pub reload: f32,
    /// The unit's unit AI could work on it (its slot 0x1b8 with 0: a priest's conversion).
    pub workable: bool,
    /// It is the unit AI's current target (`+0x48`).
    pub current: bool,
}

/// `bestUnitToAttack`'s statistics (the static table at `DAT_143e744d0`).
#[derive(Clone, Debug, PartialEq)]
pub struct Statistics {
    /// `+0`, `+4`: the least and the most distance.
    pub distance: [f32; 2],
    /// `+8`, `+0xc`: the least and the most kill time.
    pub kill_time: [f32; 2],
    /// `+0x10`, `+0x14`: the moving objects' least and most distance.
    pub moving_distance: [f32; 2],
    /// `+0x18`, `+0x1c`: the moving objects' least and most kill time.
    pub moving_kill_time: [f32; 2],
    /// `+0x20`: the objects other than the current target.
    pub others: i32,
    /// `+0x24`: the moving objects.
    pub moving: i32,
    /// `+0x28`: the armed objects.
    pub armed: i32,
    /// `+0x2c`: the objects within the unit's range against them.
    pub in_reach: i32,
    /// `+0x30`..: the objects per attack category.
    pub by_category: [i32; 12],
    /// `+0x60`, `+0x64`: the closest moving object's distance and id.
    pub closest_moving: (f32, i32),
    /// `+0x68`.., count `+0x1668`.
    pub entries: Vec<Entry>,
    /// `+0x166c`..: the entries' order, by rank after the sort.
    pub order: Vec<usize>,
}

impl Default for Statistics {
    /// `bestUnitToAttack`'s reset of the table: the least values the largest float, the most
    /// −1, the counts 0, no closest moving object (−1 at the largest float).
    fn default() -> Self {
        Self {
            distance: [f32::MAX, -1.0],
            kill_time: [f32::MAX, -1.0],
            moving_distance: [f32::MAX, -1.0],
            moving_kill_time: [f32::MAX, -1.0],
            others: 0,
            moving: 0,
            armed: 0,
            in_reach: 0,
            by_category: [0; 12],
            closest_moving: (f32::MAX, -1),
            entries: Vec::new(),
            order: Vec::new(),
        }
    }
}

/// `low` takes `v` when above it (`COMISS low, v`, `JBE`: a NaN changes nothing).
fn least(low: &mut f32, v: f32) {
    if *low > v {
        *low = v;
    }
}

/// `high` takes `v` when below it (`COMISS v, high`, `JBE`).
fn most(high: &mut f32, v: f32) {
    if v > *high {
        *high = v;
    }
}

impl Statistics {
    /// `FUN_140c66590`: the object `c` joins the table, while it holds fewer than
    /// [`STATISTICS`] (the caller's test that both have a unit AI, `+0x6b0`): its id, distance
    /// and category; the category's count when not −1; the count of others unless it is the
    /// current target; a moving one counts and may be the closest moving object (`COMISS`, a
    /// strictly nearer one); a fixed class marks it; an armed one counts, as does one within
    /// the unit's reach (`COMISS`, `JC`: a NaN reach does not); the distances' bounds; its kill
    /// time: hit points over damage over the reload time for damage above [`EPSILON`] (`JC`:
    /// a NaN damage too), else 5 for one the unit's AI could work on and the largest float for
    /// another; the kill times' bounds; the current target's bonus 5; a moving one's bounds of
    /// both.
    pub fn add(&mut self, c: &Candidate) {
        if STATISTICS <= self.entries.len() {
            return;
        }
        let mut e = Entry {
            id: c.id,
            distance: c.distance,
            category: c.category,
            ..Entry::default()
        };
        if let Some(n) = usize::try_from(c.category)
            .ok()
            .and_then(|k| self.by_category.get_mut(k))
        {
            *n += 1;
        }
        if !c.current {
            self.others += 1;
        }
        if 0.0 < c.speed {
            e.moving = true;
            if self.closest_moving.0 > c.distance {
                self.closest_moving = (c.distance, c.id);
            }
            self.moving += 1;
        }
        let class = i32::from(c.class) as u32;
        e.fixed = class <= 0x34 && FIXED_CLASSES >> class & 1 != 0;
        if c.armed {
            self.armed += 1;
        }
        if !(c.reach < c.distance || c.reach.is_nan() || c.distance.is_nan()) {
            self.in_reach += 1;
        }
        least(&mut self.distance[0], c.distance);
        most(&mut self.distance[1], c.distance);
        e.kill_time = if crate::fmath::below_or_unordered(EPSILON, c.damage) {
            (c.hp / c.damage) / c.reload
        } else if c.workable {
            NO_DAMAGE_KILL
        } else {
            NEVER
        };
        least(&mut self.kill_time[0], e.kill_time);
        most(&mut self.kill_time[1], e.kill_time);
        e.bonus = if c.current { NO_DAMAGE_KILL } else { 0.0 };
        if 0.0 < c.speed {
            least(&mut self.moving_distance[0], c.distance);
            most(&mut self.moving_distance[1], c.distance);
            least(&mut self.moving_kill_time[0], e.kill_time);
            most(&mut self.moving_kill_time[1], e.kill_time);
        }
        self.entries.push(e);
    }

    /// `FUN_140c48150`: every entry's scores, with the order history's `weights` by category and
    /// the unit's owner's targeting mode and factor (`player+0x614`, `+0x618`). The kill-time
    /// score's top is 10 (times the factor per cent in mode 1); it falls by the entry's share of
    /// the kill times' span (a moving entry's of the moving objects') when the span is not below
    /// [`EPSILON`] (`COMISS`, `JA`: a NaN span shares). The distance score is 75 less the
    /// entry's share of 75 the same way; a still entry's distance span below the epsilon gives
    /// 75 itself. The weight is the category's times 10 (0 without a category). The total is the
    /// sum with the bonus; in mode 2 an entry whose shown armor (`armored`, slot 0x870's first)
    /// is at least 100 takes the factor per cent of it.
    ///
    /// # Errors
    ///
    /// What `armored` does not model.
    pub fn score(
        &mut self,
        weights: &[f32; 12],
        (mode, factor): (i32, i32),
        armored: &mut dyn FnMut(i32) -> Result<bool, crate::GenieError>,
    ) -> Result<(), crate::GenieError> {
        let kill_span = self.kill_time[1] - self.kill_time[0];
        let distance_span = self.distance[1] - self.distance[0];
        let moving_kill_span = self.moving_kill_time[1] - self.moving_kill_time[0];
        let moving_distance_span = self.moving_distance[1] - self.moving_distance[0];
        let (kill_low, distance_low) = (self.kill_time[0], self.distance[0]);
        let (moving_kill_low, moving_distance_low) =
            (self.moving_kill_time[0], self.moving_distance[0]);
        for e in &mut self.entries {
            let top = if mode == 1 {
                factor as f32 * PER_CENT
            } else {
                1.0
            };
            let mut kill = top * SCORE;
            if e.moving {
                if crate::fmath::at_most_or_unordered(EPSILON, moving_kill_span) {
                    kill -= ((e.kill_time - moving_kill_low) / moving_kill_span) * kill;
                }
                e.kill_score = kill;
                let mut d = DISTANCE_SCORE;
                if crate::fmath::at_most_or_unordered(EPSILON, moving_distance_span) {
                    d -= ((e.distance - moving_distance_low) / moving_distance_span)
                        * DISTANCE_SCORE;
                }
                e.distance_score = d;
            } else {
                if crate::fmath::at_most_or_unordered(EPSILON, kill_span) {
                    kill -= ((e.kill_time - kill_low) / kill_span) * kill;
                }
                e.kill_score = kill;
                e.distance_score = if EPSILON > distance_span {
                    DISTANCE_SCORE
                } else {
                    DISTANCE_SCORE - ((e.distance - distance_low) / distance_span) * DISTANCE_SCORE
                };
            }
            e.weight = usize::try_from(e.category)
                .ok()
                .and_then(|k| weights.get(k))
                .map_or(0.0, |w| w * SCORE);
            e.total = ((e.kill_score + e.distance_score) + e.weight) + e.bonus;
            if mode == 2 && armored(e.id)? {
                e.total *= factor as f32 * PER_CENT;
            }
        }
        Ok(())
    }

    /// `bestUnitToAttack`'s ranks: each entry's order slot holds its index, and its rank is its
    /// tier (`tier`) times [`TIER`] plus its total.
    ///
    /// # Errors
    ///
    /// What `tier` does not model.
    pub fn rank(
        &mut self,
        tier: &mut dyn FnMut(&Entry) -> Result<i32, crate::GenieError>,
    ) -> Result<(), crate::GenieError> {
        self.order = (0..self.entries.len()).collect();
        for e in &mut self.entries {
            e.rank = tier(e)? as f32 * TIER + e.total;
        }
        Ok(())
    }

    /// `FUN_140c41830` over the order slots `lo..=hi`: the last one's entry is the pivot; each
    /// slot before it whose entry ranks above it, or the same with a lower id, moves to the
    /// front part (a NaN rank never does); then the pivot follows the front part. Its slot.
    fn partition(&mut self, lo: usize, hi: usize) -> usize {
        let pivot = self.entries[self.order[hi]];
        let mut i = lo;
        for j in lo..hi {
            let e = self.entries[self.order[j]];
            if pivot.rank < e.rank || (e.rank == pivot.rank && e.id < pivot.id) {
                self.order.swap(i, j);
                i += 1;
            }
        }
        self.order.swap(i, hi);
        i
    }

    /// `FUN_140c41b40` over the order slots `lo..=hi` (the quicksort, its second half by the
    /// loop).
    fn quicksort(&mut self, mut lo: usize, hi: usize) {
        while lo < hi {
            let p = self.partition(lo, hi);
            if p > 0 {
                self.quicksort(lo, p - 1);
            }
            lo = p + 1;
        }
    }

    /// `bestUnitToAttack`'s sort of the order slots by rank, the highest first (ties to the
    /// lower id): with more than one entry, `FUN_140c41830` over all, then `FUN_140c41b40` over
    /// both sides of the pivot.
    pub fn sort(&mut self) {
        let Some(last) = self.entries.len().checked_sub(1).filter(|&l| 0 < l) else {
            return;
        };
        let p = self.partition(0, last);
        if p > 0 {
            self.quicksort(0, p - 1);
        }
        self.quicksort(p + 1, last);
    }
}

/// The order history's weights by attack category (`bestUnitToAttack`'s walk of `+0xa8` from
/// the newest): each order younger than a minute (`now` less its time, unsigned, at most 60000)
/// adds the rest of the minute over 60000 (`DAT_142efe420`) to its target's category; the first
/// older one ends the walk.
#[must_use]
pub fn order_weights(history: &[crate::unit_ai::OrderRecord], now: u32) -> [f32; 12] {
    let mut w = [0.0f32; 12];
    for r in history {
        let age = now.wrapping_sub(r.time);
        if 60000 < age {
            break;
        }
        if let Some(k) = usize::try_from(r.category).ok().filter(|&k| k < 12) {
            w[k] += (60000 - age) as f32 / ORDER_WINDOW;
        }
    }
    w
}

/// The order history's window (`DAT_142efe420`, 60000 ms as a float).
const ORDER_WINDOW: f32 = crate::image::best_target::ORDER_WINDOW;

/// What `bestUnitToAttack` reads of the unit whose AI searches, and of that AI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Searcher {
    /// The unit.
    pub id: i32,
    /// Its unit AI's current target (`+0x48`), −1 for none.
    pub target: i32,
    /// The AI's target's class (`+0x4c`) and its last target's (`+0x70`).
    pub target_types: [i32; 2],
    /// The AI's order (`+0x3c`) and action (`+0x44`).
    pub order: i32,
    /// The AI's action (`+0x44`).
    pub action: i32,
    /// The unit's master's id, class and type (`+0x5d7`, `+0x98f`, the master's `+0x8`).
    pub master: i16,
    /// Its master's class.
    pub class: i16,
    /// Its master's type (`'F'`, 70: the combat type).
    pub kind: u8,
    /// Its master's `attack_priority` (`+0x92c`, `FUN_141202220`).
    pub attack_priority: u8,
    /// Its owner is a computer player (`FUN_141659cf0`).
    pub computer: bool,
    /// Its owner may attack walls (`player+0x611`, the AI's slot 0x18's player).
    pub walls_allowed: bool,
    /// The AI's slot 0x1d0 with −1: the unit may move to attack.
    pub can_move: bool,
    /// `FUN_140c47f90`: the farthest it looks (a human player's twice its line of sight).
    pub sight: f32,
    /// Its weapon range against its current target (slot 0x238 with it, or with none).
    pub range: f32,
    /// Its weapon range (slot 0x230) and least range (slot 0x240).
    pub weapon_range: f32,
    /// Its least range (slot 0x240).
    pub min_range: f32,
    /// Its top speed (slot 0x208).
    pub speed: f32,
    /// The game mode is a scenario's (`FUN_141302540`).
    pub scenario: bool,
    /// Its owner's targeting mode and factor (`player+0x614`, `+0x618`).
    pub targeting: (i32, i32),
    /// The order history's weights ([`order_weights`]).
    pub weights: [f32; 12],
    /// The AI's retarget entries' targets (`+0x1e0`).
    pub retries: Vec<i32>,
    /// The world's time (`FUN_1415300d0`).
    pub now: u32,
}

/// `bestUnitToAttack`'s arguments past its reference point (the search's, the caller's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ask {
    /// The reference distance (0: none).
    pub distance: f32,
    /// An idle check: the current target narrows nothing and is no shortcut.
    pub idle: bool,
    /// The path checks: 1 on, 0 off (the shortcuts answer without a path).
    pub paths: u8,
    /// Extra objects for the statistics (`int*`, count).
    pub extra: Vec<i32>,
    /// Any class: walls, gates, buildings, towers and packed units are not passed over.
    pub any_class: bool,
    /// The unit is in an attack work state: a current target on top answers at once.
    pub work_state: bool,
}

/// What `bestUnitToAttack` reads of an object of the world (`FUN_1415417a0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thing {
    /// Its owner's id.
    pub owner: usize,
    /// The unit's owner counts its owner an ally (`FUN_141663d90` with the ally flag).
    pub allied: bool,
    /// `+0x640`.
    pub state: u8,
    /// It has a unit AI (`+0x6b0`).
    pub has_ai: bool,
    /// It is inside another (`+0x2f0`).
    pub inside: bool,
    /// Its master's id, class and type.
    pub master: i16,
    /// Its master's class.
    pub class: i16,
    /// Its master's type.
    pub kind: u8,
    /// Its `+0x924` (`FUN_14107cc90`): others take it over by seeing it.
    pub capture: bool,
    /// Its targeting flag (`+0x689`, `FUN_141188bb0`).
    pub targetable: bool,
    /// Its attackable flag (`+0x68a`, `FUN_141188b10`).
    pub attackable: bool,
    /// What it holds (`+0x310`).
    pub held: f32,
    /// The footprints' gap to the unit (`FUN_14118f680`).
    pub distance: f32,
    /// Its top speed (slot 0x208).
    pub speed: f32,
}

/// What `bestUnitToAttack` asks of the world while its unit AI runs.
pub trait Battlefield {
    /// The object `id` as the world holds it, None for one it does not.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold.
    fn thing(&mut self, id: i32) -> Result<Option<Thing>, crate::GenieError>;

    /// The unit AI's attack check (slot 0xa0 with 0) on object `id`.
    ///
    /// # Errors
    ///
    /// What the check does not model.
    fn can_attack(&mut self, id: i32) -> Result<bool, crate::GenieError>;

    /// The unit AI's neutral test (slot 0xa8) of an object of `class`.
    ///
    /// # Errors
    ///
    /// What the test does not model.
    fn attacks_neutral(&mut self, class: i16) -> Result<bool, crate::GenieError>;

    /// `FUN_14120e4e0` of a unit of the combat type on object `id`: it spares its friends.
    ///
    /// # Errors
    ///
    /// What the test does not model.
    fn spares_friends(&mut self, id: i32) -> Result<bool, crate::GenieError>;

    /// `FUN_140c41690`: a computer player's unit passes object `id` over.
    ///
    /// # Errors
    ///
    /// What the test does not model.
    fn shuns(&mut self, id: i32) -> Result<bool, crate::GenieError>;

    /// What the statistics read of object `id` at `distance` of `category`
    /// ([`Candidate`]).
    ///
    /// # Errors
    ///
    /// What the reads do not model.
    fn candidate(
        &mut self,
        id: i32,
        distance: f32,
        category: i32,
    ) -> Result<Candidate, crate::GenieError>;

    /// The shown armor of object `id` is at least 100 (slot 0x870's first).
    ///
    /// # Errors
    ///
    /// What the read does not model.
    fn armored(&mut self, id: i32) -> Result<bool, crate::GenieError>;

    /// Object `id`'s weapon range (slot 0x230).
    ///
    /// # Errors
    ///
    /// What the read does not model.
    fn weapon_range(&mut self, id: i32) -> Result<f32, crate::GenieError>;

    /// The current target `id` engages: its attacks sum above 0 (slot 0x228) or its unit AI
    /// could work on the unit (slot 0x1b8 with 1).
    ///
    /// # Errors
    ///
    /// What the reads do not model.
    fn engages(&mut self, id: i32) -> Result<bool, crate::GenieError>;

    /// The unit's quick path to object `id` within `range` (slot 0x3e0).
    ///
    /// # Errors
    ///
    /// What the path does not model.
    fn quick_path(&mut self, id: i32, range: f32) -> Result<bool, crate::GenieError>;

    /// The unit's path to object `id` within `range` (slot 0x3d0).
    ///
    /// # Errors
    ///
    /// What the path does not model.
    fn path(&mut self, id: i32, range: f32) -> Result<bool, crate::GenieError>;
}

/// The boats' classes (2, 20, 21, 22, 53), which a computer player's land unit passes over.
const BOATS: [i16; 5] = [2, 20, 21, 22, 53];
/// The classes a search without `any_class` passes over: buildings, walls, towers, gates and
/// the packed units.
const NOT_ANY: [i16; 5] = [3, 27, 52, 39, 51];
/// The wall's and the gate's classes.
const WALL: i16 = 27;
const GATE: i16 = 39;
/// The masters whose units attack by priority 0 without the moving rule: the rams, the siege
/// masters 1744 and 1746, 420 and 691.
const PRIORITY_0_MASTERS: [i16; 8] = [35, 422, 548, 1258, 1744, 1746, 420, 691];
/// A computer player's unit of these masters passes no boat over: the castle, 45, the town
/// centre.
const BOAT_HUNTERS: [i16; 3] = [82, 45, 109];
/// The moving targets past which the current target and the closest moving one answer.
const FEW_MOVING: i32 = 5;
/// The orders of the defend patrol (`0x2dd`) and class 13 (siege), whose units keep the walls'
/// owner flag.
const DEFEND_PATROL: i32 = 0x2dd;
const SIEGE: i16 = 13;
/// The retarget entries' times: a moving object's 1 s, another's 2 s.
const RETRY_MOVING: u32 = 1000;
const RETRY_OTHER: u32 = 2000;

/// What `bestUnitToAttack` found: the object to attack (−1 for none) and the retarget entries
/// its failed paths add (`FUN_140c42320`: each target and the world time until which it is
/// ignored), the caller's for the unit AI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    /// The object, −1 for none.
    pub target: i32,
    /// The retarget entries to add, in order.
    pub retries: Vec<(i32, u32)>,
}

/// The object tests the statistics' main and retry walks share: the world holds it with a unit
/// AI, alive, not allied, not inside another; the unit AI's attack check; a combat unit's
/// friends' scan; a neutral object's test; a Gaia unit others take by sight is passed outside
/// the scenarios; its flags; a computer player's tests; the classes without `any_class`; a
/// computer player's unit's Gaia holdings and boats. The object and its category, or None.
fn tested(
    me: &Searcher,
    ask: &Ask,
    id: i32,
    kind: usize,
    world: &mut dyn Battlefield,
) -> Result<Option<Thing>, crate::GenieError> {
    let Some(t) = world.thing(id)? else {
        return Ok(None);
    };
    if !t.has_ai || 3 <= t.state || t.allied || t.inside || !world.can_attack(id)? {
        return Ok(None);
    }
    if me.kind == 70 && !world.spares_friends(id)? {
        return Ok(None);
    }
    if kind == 3 && !world.attacks_neutral(t.class)? {
        return Ok(None);
    }
    let gaia_taken = !me.scenario && t.owner == 0 && 0x45 < t.kind;
    if gaia_taken && t.capture {
        return Ok(None);
    }
    if !t.targetable || (!t.attackable && me.action == 600) || world.shuns(id)? {
        return Ok(None);
    }
    if !ask.any_class && NOT_ANY.contains(&t.class) {
        return Ok(None);
    }
    let captured = 0x45 < t.kind && t.capture;
    if me.computer && t.owner == 0 {
        // nothing held (`+0x310` at most 0) and not taken by sight
        let free = t.held <= 0.0 && !captured;
        if !free {
            return Ok(None);
        }
    }
    let boat_free =
        me.class == 52 || BOAT_HUNTERS.contains(&me.master) || !BOATS.contains(&t.class);
    let is_boat = BOATS.contains(&me.class);
    if me.computer && !boat_free && !is_boat {
        return Ok(None);
    }
    Ok(Some(t))
}

/// The ranges the statistics' walks test an object at `gap` by: at least the least range,
/// within the weapon range unless the unit moves, within `near` (the main walk's sight, the
/// retry walk's range) unless it may move, within the reference distance when there is one, and
/// within the sight.
fn within(me: &Searcher, ask: &Ask, gap: f32, near: f32) -> bool {
    me.min_range <= gap
        && (gap <= me.weapon_range || me.speed != 0.0)
        && (me.can_move || gap <= near)
        && (ask.distance <= 0.0 || gap <= ask.distance)
        && gap <= me.sight
}

/// `UnitAIModule::bestUnitToAttack` (`FUN_140c44680`) for the unit `me`, over what its search
/// (slot 0x50, the caller's) left in the relation lists `lists` (0 Gaia .. 4 enemies). The
/// statistics ([`Statistics`]) take:
///
/// - the current target (the world holds it alive, not allied) when within the unit's range
///   against it or the unit moves; outside an idle check, one within the reference distance
///   that is no building (3) nor farm (49) narrows the search to its category;
/// - the extra objects but the current target, by the attack check, one under the least range
///   at its gap plus the weapon range plus 1 (none for a unit that does not move);
/// - each list from enemies down to Gaia's (a human player's down to 2 in a scenario), each
///   record but the current target and the extra objects: a retarget entry's target waits for
///   the retry walk; the others pass the shared tests ([`tested`]), then the ranges (the least
///   range, the weapon range unless the unit moves, the sight unless it may move, the reference
///   distance, the sight), the category; the closest gate and the closest wall are noted, and
///   count only for a unit that attacks walls (its targets' classes, or its owner's flag
///   outside the defend patrol of a unit that is no siege).
///
/// With walls to attack and nothing but walls, the closest gate answers. Without statistics the
/// retry walk adds its targets the same way (by the unit's range, not the sight, unless it
/// may move); still none, the closest wall answers (a unit that attacks walls) or −1. Then the
/// scores ([`Statistics::score`]), the ranks (a fixed entry but category 6 by its weapon range:
/// 3 above the epsilon, else 2, a farm 1; the others 4 for a unit of priority 1 or of 0 but
/// [`PRIORITY_0_MASTERS`], else 0) and the sort. Fewer than 5 moving entries and a current
/// target that moves (or nothing moves at all), outside an idle check, when it engages or
/// nothing is armed: the current target, by its path (without path checks at once). Then the
/// first three entries by the quick path (a moving one, or with nothing moving or a unit of
/// another rank rule); more than 5 moving entries outside an idle check: the closest moving one
/// by its path; in an attack work state a current target on top; without path checks the top;
/// else the first three by their paths, each one that fails ignored for 1 s (moving) or 2 s.
/// Last the closest wall (a unit that attacks walls) or −1.
///
/// # Errors
///
/// What the world's reads and tests do not model.
pub fn best_unit_to_attack(
    me: &Searcher,
    ask: &Ask,
    lists: &[Vec<i32>; 5],
    world: &mut dyn Battlefield,
) -> Result<Choice, crate::GenieError> {
    let mut choice = Choice::default();
    let defending = me.order == DEFEND_PATROL && me.class != SIEGE;
    let current = if me.target == -1 {
        None
    } else {
        world
            .thing(me.target)?
            .filter(|t| t.state <= 2 && !t.allied)
    };
    let mut st = Statistics::default();
    let walls = matches!(me.target_types[0], 27 | 39)
        || matches!(me.target_types[1], 27 | 39)
        || (me.walls_allowed && !defending);
    let range = me.range;
    let mut narrowed = -1;
    if let Some(t) = current.filter(|t| t.distance <= range || 0.0 < me.speed) {
        let k = category(t.master, t.class);
        st.add(&world.candidate(me.target, t.distance, k)?);
        if !ask.idle && t.distance <= ask.distance && !matches!(t.class, 3 | 49) {
            narrowed = k;
        }
    }
    for &id in &ask.extra {
        if id == me.target {
            continue;
        }
        let Some(t) = world.thing(id)? else {
            continue;
        };
        if !world.can_attack(id)? {
            continue;
        }
        let mut gap = t.distance;
        if gap < me.min_range {
            if me.speed == 0.0 {
                continue;
            }
            gap = gap + me.weapon_range + ONE;
        }
        st.add(&world.candidate(id, gap, category(t.master, t.class))?);
    }
    let lowest = if me.scenario && !me.computer { 2 } else { 0 };
    let mut later = Vec::new();
    let (mut gate, mut gate_gap) = (-1, -1.0f32);
    let (mut wall, mut wall_gap) = (-1, -1.0f32);
    let mut beyond_walls = false;
    'lists: for kind in (lowest..=4).rev() {
        for &id in &lists[kind] {
            if STATISTICS <= st.entries.len() {
                break 'lists;
            }
            if id == me.target || ask.extra.contains(&id) {
                continue;
            }
            if me.retries.contains(&id) {
                later.push((id, kind));
                continue;
            }
            let Some(t) = tested(me, ask, id, kind, world)? else {
                continue;
            };
            let gap = t.distance;
            if !within(me, ask, gap, me.sight) {
                continue;
            }
            let k = category(t.master, t.class);
            if narrowed != -1 && k != narrowed {
                continue;
            }
            if t.class == GATE {
                if gate_gap < 0.0 || gap < gate_gap {
                    (gate, gate_gap) = (id, gap);
                }
                if !walls {
                    continue;
                }
            } else if t.class == WALL {
                if wall_gap < 0.0 || gap < wall_gap {
                    (wall, wall_gap) = (id, gap);
                }
                if !walls {
                    continue;
                }
            } else {
                beyond_walls = true;
            }
            st.add(&world.candidate(id, gap, k)?);
        }
    }
    if walls && !beyond_walls && gate != -1 {
        choice.target = gate;
        return Ok(choice);
    }
    if st.entries.is_empty() {
        for &(id, kind) in &later {
            let Some(t) = tested(me, ask, id, kind, world)? else {
                continue;
            };
            let gap = t.distance;
            if !within(me, ask, gap, range) {
                continue;
            }
            let k = category(t.master, t.class);
            if narrowed != -1 && k != narrowed {
                continue;
            }
            if t.class == GATE {
                if !walls {
                    continue;
                }
            } else if t.class == WALL {
                if wall_gap < 0.0 || gap < wall_gap {
                    (wall, wall_gap) = (id, gap);
                }
                if !walls {
                    continue;
                }
            }
            st.add(&world.candidate(id, gap, k)?);
        }
        if st.entries.is_empty() {
            choice.target = if walls { wall } else { -1 };
            return Ok(choice);
        }
    }
    st.score(&me.weights, me.targeting, &mut |id| world.armored(id))?;
    let ranked = me.attack_priority == 1
        || (me.attack_priority == 0 && !PRIORITY_0_MASTERS.contains(&me.master));
    st.rank(&mut |e| {
        if !e.fixed || e.category == 6 {
            return Ok(i32::from(ranked) << 2);
        }
        if EPSILON < world.weapon_range(e.id)? {
            return Ok(3);
        }
        let class = world.thing(e.id)?.map_or(-1, |t| t.class);
        Ok(if class == 49 { 1 } else { 2 })
    })?;
    st.sort();
    if let Some(t) = current
        && st.moving < FEW_MOVING
        && ((t.speed < EPSILON && st.moving == 0) || 0.0 < t.speed)
        && !ask.idle
        && (world.engages(me.target)? || st.armed < 1)
        && (ask.paths == 0 || world.path(me.target, range)?)
    {
        choice.target = me.target;
        return Ok(choice);
    }
    for &i in st.order.iter().take(3) {
        let e = st.entries[i];
        if (e.moving || st.moving < 1 || !ranked) && world.quick_path(e.id, range)? {
            choice.target = e.id;
            return Ok(choice);
        }
    }
    let mut tried = -1;
    if !ask.idle && FEW_MOVING < st.moving {
        let closest = st.closest_moving.1;
        if ask.paths == 0 || world.path(closest, range)? {
            choice.target = closest;
            return Ok(choice);
        }
        tried = closest;
    }
    let top = st.entries[st.order[0]].id;
    if ask.work_state && top == me.target {
        choice.target = me.target;
        return Ok(choice);
    }
    if ask.paths == 0 {
        choice.target = top;
        return Ok(choice);
    }
    for &i in st.order.iter().take(3) {
        let e = st.entries[i];
        if e.id != tried && world.path(e.id, range)? {
            choice.target = e.id;
            return Ok(choice);
        }
        let wait = if e.moving { RETRY_MOVING } else { RETRY_OTHER };
        choice.retries.push((e.id, me.now.wrapping_add(wait)));
    }
    choice.target = if walls { wall } else { -1 };
    Ok(choice)
}

/// 1 (`DAT_14330eb8c`): an extra object under the least range counts this much beyond the
/// weapon range.
const ONE: f32 = crate::image::best_target::ONE;

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i32, distance: f32, speed: f32) -> Candidate {
        Candidate {
            id,
            distance,
            category: 1,
            class: 6,
            speed,
            armed: true,
            reach: 1.0,
            damage: 4.0,
            hp: 40.0,
            reload: 2.0,
            workable: false,
            current: false,
        }
    }

    /// `FUN_140c66590`: the bounds, the counts, the closest moving object, the kill time (hit
    /// points over damage over reload: 40 / 4 / 2 = 5), 5 or the largest float without damage,
    /// the fixed classes and the current target's bonus.
    #[test]
    fn the_statistics_count_bound_and_time_each_object() {
        let mut st = Statistics::default();
        st.add(&candidate(1, 3.0, 1.0));
        st.add(&Candidate {
            current: true,
            class: 3,
            damage: 0.0,
            workable: true,
            ..candidate(2, 0.5, 0.0)
        });
        st.add(&Candidate {
            damage: 0.0,
            category: -1,
            armed: false,
            ..candidate(3, 7.0, 2.0)
        });
        assert_eq!(st.distance, [0.5, 7.0]);
        assert_eq!(st.kill_time, [5.0, f32::MAX]);
        assert_eq!(st.moving_distance, [3.0, 7.0]);
        assert_eq!(st.moving_kill_time, [5.0, f32::MAX]);
        assert_eq!((st.others, st.moving, st.armed, st.in_reach), (2, 2, 2, 1));
        assert_eq!(st.by_category[1], 2);
        assert_eq!(st.closest_moving, (3.0, 1));
        let kill: Vec<f32> = st.entries.iter().map(|e| e.kill_time).collect();
        assert_eq!(kill, [5.0, 5.0, f32::MAX]);
        let fixed: Vec<bool> = st.entries.iter().map(|e| e.fixed).collect();
        assert_eq!(fixed, [false, true, false]);
        assert_eq!(st.entries[1].bonus, 5.0);
    }

    /// `FUN_140c48150`, then the rank and the sort: a still entry alone in its kind takes the
    /// whole kill score (10) and, its distance span below the epsilon, 75; moving entries share
    /// their spans; the weight is its category's times 10. The sort ranks the highest first,
    /// a tie to the lower id.
    #[test]
    fn the_scores_rank_and_sort_the_entries() {
        let mut st = Statistics::default();
        st.add(&candidate(5, 2.0, 1.0));
        st.add(&Candidate {
            hp: 80.0,
            ..candidate(4, 6.0, 1.0)
        });
        st.add(&candidate(9, 4.0, 0.0));
        let mut weights = [0.0; 12];
        weights[1] = 0.5;
        st.score(&weights, (0, 0), &mut |_| Ok(false)).unwrap();
        let scores: Vec<(f32, f32, f32, f32)> = st
            .entries
            .iter()
            .map(|e| (e.kill_score, e.distance_score, e.weight, e.total))
            .collect();
        // moving kill times 5 and 10: 10 - (0/5)·10 = 10 and 10 - (5/5)·10 = 0; moving
        // distances 2 and 6: 75 and 0; the still entry: kill span 10 - 5 over all, distance
        // span 6 - 2 over all
        assert_eq!(
            scores,
            [
                (10.0, 75.0, 5.0, 90.0),
                (0.0, 0.0, 5.0, 5.0),
                (10.0, 37.5, 5.0, 52.5)
            ]
        );
        st.rank(&mut |e| Ok(i32::from(!e.moving))).unwrap();
        st.sort();
        let ids: Vec<i32> = st.order.iter().map(|&i| st.entries[i].id).collect();
        assert_eq!(ids, [9, 5, 4]);
        // equal ranks go to the lower id
        for e in &mut st.entries {
            e.rank = 1.0;
        }
        st.order = vec![0, 1, 2];
        st.sort();
        let ids: Vec<i32> = st.order.iter().map(|&i| st.entries[i].id).collect();
        assert_eq!(ids, [4, 5, 9]);
    }

    /// The order history's weights: the younger an order, the more its category weighs; the
    /// first one older than a minute ends the walk.
    #[test]
    fn the_order_history_weighs_the_last_minute_s_categories() {
        let record = |time, category| crate::unit_ai::OrderRecord {
            time,
            category,
            ..crate::unit_ai::OrderRecord::default()
        };
        let w = order_weights(
            &[
                record(100_000, 1),
                record(70_000, 1),
                record(30_000, 3),
                record(99_000, 2),
            ],
            100_000,
        );
        assert_eq!(w[1], 1.0 + 30_000.0 / 60_000.0);
        assert_eq!(w[3], 0.0);
        assert_eq!(w[2], 0.0);
    }

    /// A world of objects by id: each a [`Thing`] and its candidate's values; the paths that
    /// succeed and the calls made.
    #[derive(Default)]
    struct Field {
        things: std::collections::BTreeMap<i32, (Thing, Candidate)>,
        quick: Vec<i32>,
        paths: Vec<i32>,
        asked: Vec<(char, i32)>,
    }

    impl Battlefield for Field {
        fn thing(&mut self, id: i32) -> Result<Option<Thing>, crate::GenieError> {
            Ok(self.things.get(&id).map(|t| t.0))
        }
        fn can_attack(&mut self, _: i32) -> Result<bool, crate::GenieError> {
            Ok(true)
        }
        fn attacks_neutral(&mut self, _: i16) -> Result<bool, crate::GenieError> {
            Ok(true)
        }
        fn spares_friends(&mut self, _: i32) -> Result<bool, crate::GenieError> {
            Ok(true)
        }
        fn shuns(&mut self, _: i32) -> Result<bool, crate::GenieError> {
            Ok(false)
        }
        fn candidate(
            &mut self,
            id: i32,
            distance: f32,
            category: i32,
        ) -> Result<Candidate, crate::GenieError> {
            Ok(Candidate {
                distance,
                category,
                ..self.things[&id].1
            })
        }
        fn armored(&mut self, _: i32) -> Result<bool, crate::GenieError> {
            Ok(false)
        }
        fn weapon_range(&mut self, _: i32) -> Result<f32, crate::GenieError> {
            Ok(0.0)
        }
        fn engages(&mut self, _: i32) -> Result<bool, crate::GenieError> {
            Ok(true)
        }
        fn quick_path(&mut self, id: i32, _: f32) -> Result<bool, crate::GenieError> {
            self.asked.push(('q', id));
            Ok(self.quick.contains(&id))
        }
        fn path(&mut self, id: i32, _: f32) -> Result<bool, crate::GenieError> {
            self.asked.push(('p', id));
            Ok(self.paths.contains(&id))
        }
    }

    fn soldier() -> Searcher {
        Searcher {
            id: 1,
            target: -1,
            target_types: [-1, -1],
            order: -1,
            action: -1,
            master: 74,
            class: 6,
            kind: 70,
            attack_priority: 1,
            can_move: true,
            sight: 12.0,
            range: 0.5,
            weapon_range: 0.5,
            speed: 1.0,
            now: 10_000,
            ..Searcher::default()
        }
    }

    fn enemy(id: i32, class: i16, distance: f32, hp: f32) -> (Thing, Candidate) {
        (
            Thing {
                owner: 2,
                state: 2,
                has_ai: true,
                master: 4,
                class,
                kind: 70,
                targetable: true,
                attackable: true,
                distance,
                speed: 1.0,
                ..Thing::default()
            },
            Candidate {
                speed: 1.0,
                hp,
                ..candidate(id, distance, 1.0)
            },
        )
    }

    /// `bestUnitToAttack` over three enemies (an idle check, path checks on): the statistics
    /// rank them (the nearest's distance score outweighs the weakest's kill score: 7, 9, 8); the
    /// quick paths of the first three (moving objects) are asked, 9's answers.
    #[test]
    fn the_best_unit_to_attack_ranks_then_takes_the_first_quick_path() {
        let mut field = Field::default();
        let _ = field.things.insert(7, enemy(7, 0, 3.0, 40.0));
        let _ = field.things.insert(8, enemy(8, 0, 9.0, 40.0));
        let _ = field.things.insert(9, enemy(9, 0, 5.0, 10.0));
        field.quick = vec![9];
        let lists = [vec![], vec![], vec![], vec![], vec![8, 7, 9]];
        let ask = Ask {
            idle: true,
            paths: 1,
            any_class: true,
            ..Ask::default()
        };
        let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, 9);
        assert_eq!(field.asked, [('q', 7), ('q', 9)]);
        assert_eq!(c.retries, []);
        // without a quick path the paths follow; 7's fails (a moving object: ignored for 1 s)
        field.quick.clear();
        field.paths = vec![9];
        field.asked.clear();
        let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, 9);
        assert_eq!(
            field.asked,
            [('q', 7), ('q', 9), ('q', 8), ('p', 7), ('p', 9)]
        );
        assert_eq!(c.retries, [(7, 11_000)]);
    }

    /// Each of the first three whose path fails is ignored for 1 s (a moving object) or 2 s
    /// (a still one), and nothing found answers −1. A retarget entry's target waits for the
    /// retry walk, which runs only without other statistics.
    #[test]
    fn failed_paths_become_retarget_entries() {
        let mut field = Field::default();
        let _ = field.things.insert(7, enemy(7, 0, 3.0, 40.0));
        let mut still = enemy(9, 0, 5.0, 10.0);
        still.0.speed = 0.0;
        still.1.speed = 0.0;
        let _ = field.things.insert(9, still);
        let lists = [vec![], vec![], vec![], vec![], vec![7, 9]];
        let ask = Ask {
            idle: true,
            paths: 1,
            any_class: true,
            ..Ask::default()
        };
        let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, -1);
        let mut retries = c.retries;
        retries.sort_unstable();
        assert_eq!(retries, [(7, 11_000), (9, 12_000)]);
        let me = Searcher {
            retries: vec![9],
            ..soldier()
        };
        field.asked.clear();
        let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, -1);
        assert_eq!(field.asked, [('q', 7), ('p', 7)]);
        // with 7 a retarget entry too, the retry walk takes both
        let me = Searcher {
            retries: vec![9, 7],
            ..soldier()
        };
        field.paths = vec![9];
        field.asked.clear();
        let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, 9);
    }

    /// The lists' records pass the objects without a unit AI, inside another, dead or allied,
    /// and those beyond the sight (twice the line of sight: 12) or under the least range; the
    /// one left answers.
    #[test]
    fn the_lists_pass_over_what_the_unit_may_not_attack() {
        let mut field = Field::default();
        let mut no_ai = enemy(30, 0, 3.0, 40.0);
        no_ai.0.has_ai = false;
        let mut inside = enemy(31, 0, 3.0, 40.0);
        inside.0.inside = true;
        let mut dead = enemy(32, 0, 3.0, 40.0);
        dead.0.state = 3;
        let mut ally = enemy(33, 0, 3.0, 40.0);
        ally.0.allied = true;
        for (id, t) in [(30, no_ai), (31, inside), (32, dead), (33, ally)] {
            let _ = field.things.insert(id, t);
        }
        let _ = field.things.insert(34, enemy(34, 0, 12.5, 40.0));
        let _ = field.things.insert(35, enemy(35, 0, 0.1, 40.0));
        let _ = field.things.insert(36, enemy(36, 0, 11.9, 40.0));
        field.quick = (30..=36).collect();
        let lists = [vec![], vec![], vec![], vec![], (30..=36).collect()];
        let ask = Ask {
            idle: true,
            paths: 1,
            any_class: true,
            ..Ask::default()
        };
        let me = Searcher {
            min_range: 0.2,
            ..soldier()
        };
        let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, 36);
        assert_eq!(field.asked, [('q', 36)]);
    }

    /// A unit that attacks walls (its last target a wall) and finds only walls and gates takes
    /// the closest gate; one that does not passes them over.
    #[test]
    fn walls_and_gates_count_for_a_unit_that_attacks_walls() {
        let mut field = Field::default();
        let _ = field.things.insert(20, enemy(20, 27, 2.0, 100.0));
        let _ = field.things.insert(21, enemy(21, 39, 4.0, 100.0));
        let _ = field.things.insert(22, enemy(22, 39, 3.0, 100.0));
        let lists = [vec![], vec![], vec![], vec![], vec![20, 21, 22]];
        let ask = Ask {
            idle: true,
            paths: 1,
            any_class: true,
            ..Ask::default()
        };
        let me = Searcher {
            target_types: [-1, 27],
            ..soldier()
        };
        let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, 22);
        let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
        assert_eq!(c.target, -1);
    }

    /// `FUN_140c4c8e0`'s switch: the archers' class 0 is 3, but masters 6 and 7 (the crossbowman
    /// lines' first) are 4; the siege's 13 is 6, but the rams' masters are 5; a class outside the
    /// table is −1.
    #[test]
    fn the_categories_follow_the_classes_and_their_exceptions() {
        assert_eq!(category(4, 0), 3);
        assert_eq!(category(6, 0), 4);
        assert_eq!(category(7, 23), 4);
        assert_eq!(category(83, 4), 7);
        assert_eq!(category(74, 6), 1);
        assert_eq!(category(1744, 6), 5);
        assert_eq!(category(280, 13), 6);
        assert_eq!(category(35, 13), 5);
        assert_eq!(category(1258, 55), 5);
        assert_eq!(category(109, 3), 0);
        assert_eq!(category(448, 47), 1);
        assert_eq!(category(1, 1), -1);
        assert_eq!(category(1, 63), -1);
        assert_eq!(category(1, -1), -1);
    }
}
