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
