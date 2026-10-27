//! `bestUnitToAttack` itself (`FUN_140c44680`): the object tests, the ranges and the choice.

use crate::unit_ai::best_target::{
    Ask, Battlefield, EPSILON, STATISTICS, Searcher, Statistics, Thing, category,
};

/// The boats' classes (2, 20, 21, 22, 53), which a computer player's land unit passes over.
pub(super) const BOATS: [i16; 5] = [2, 20, 21, 22, 53];
/// The classes a search without `any_class` passes over: buildings, walls, towers, gates and
/// the packed units.
pub(super) const NOT_ANY: [i16; 5] = [3, 27, 52, 39, 51];
/// The wall's and the gate's classes.
pub(super) const WALL: i16 = 27;
pub(super) const GATE: i16 = 39;
/// The masters whose units attack by priority 0 without the moving rule: the rams, the siege
/// masters 1744 and 1746, 420 and 691.
pub(super) const PRIORITY_0_MASTERS: [i16; 8] = [35, 422, 548, 1258, 1744, 1746, 420, 691];
/// A computer player's unit of these masters passes no boat over: the castle, 45, the town
/// centre.
pub(super) const BOAT_HUNTERS: [i16; 3] = [82, 45, 109];
/// The moving targets past which the current target and the closest moving one answer.
pub(super) const FEW_MOVING: i32 = 5;
/// The orders of the defend patrol (`0x2dd`) and class 13 (siege), whose units keep the walls'
/// owner flag.
pub(super) const DEFEND_PATROL: i32 = 0x2dd;
pub(super) const SIEGE: i16 = 13;
/// The retarget entries' times: a moving object's 1 s, another's 2 s.
pub(super) const RETRY_MOVING: u32 = 1000;
pub(super) const RETRY_OTHER: u32 = 2000;

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

/// The object tests the statistics' main and retry walks share: the world's attack checks
/// ([`may_attack`]); a Gaia unit others take by sight is passed outside the scenarios; its
/// flags and whether the unit shuns it; then the owner and class rules
/// ([`kept_by_owner_and_class`]). The object and its category, or None.
pub(super) fn tested(
    me: &Searcher,
    ask: &Ask,
    id: i32,
    kind: usize,
    world: &mut dyn Battlefield,
) -> Result<Option<Thing>, crate::GenieError> {
    let Some(t) = world.thing(id)? else {
        return Ok(None);
    };
    if !may_attack(me, (id, &t), kind, world)? {
        return Ok(None);
    }
    let gaia_taken = !me.scenario && t.owner == 0 && 0x45 < t.kind;
    if gaia_taken && t.capture {
        return Ok(None);
    }
    if !t.targetable || (!t.attackable && me.action == 600) || world.shuns(id)? {
        return Ok(None);
    }
    Ok(kept_by_owner_and_class(me, ask, &t).then_some(t))
}

/// The world's checks of object `id` (`t`, of relation list `kind`), in the game's order: it
/// has a unit AI, is alive, not allied and not inside another; the unit AI's attack check; a
/// combat unit's friends' scan; a neutral object's (list 3) test.
pub(super) fn may_attack(
    me: &Searcher,
    (id, t): (i32, &Thing),
    kind: usize,
    world: &mut dyn Battlefield,
) -> Result<bool, crate::GenieError> {
    if !t.has_ai || 3 <= t.state || t.allied || t.inside || !world.can_attack(id)? {
        return Ok(false);
    }
    if me.kind == 70 && !world.spares_friends(id)? {
        return Ok(false);
    }
    if kind == 3 && !world.attacks_neutral(t.class)? {
        return Ok(false);
    }
    Ok(true)
}

/// The owner and class rules of an object `t`: without `any_class` the buildings, walls,
/// towers, gates and packed units are passed over; a computer player's unit passes over Gaia's
/// objects that hold something or that others take by sight, and boats unless it is a boat, of
/// class 52 or of [`BOAT_HUNTERS`].
pub(super) fn kept_by_owner_and_class(me: &Searcher, ask: &Ask, t: &Thing) -> bool {
    if !ask.any_class && NOT_ANY.contains(&t.class) {
        return false;
    }
    let captured = 0x45 < t.kind && t.capture;
    // nothing held (`+0x310` at most 0) and not taken by sight
    let free = t.held <= 0.0 && !captured;
    if me.computer && t.owner == 0 && !free {
        return false;
    }
    let boat_free =
        me.class == 52 || BOAT_HUNTERS.contains(&me.master) || !BOATS.contains(&t.class);
    let is_boat = BOATS.contains(&me.class);
    !(me.computer && !boat_free && !is_boat)
}

/// The ranges the statistics' walks test an object at `gap` by: at least the least range,
/// within the weapon range unless the unit moves, within `near` (the main walk's sight, the
/// retry walk's range) unless it may move, within the reference distance when there is one, and
/// within the sight.
pub(super) fn within(me: &Searcher, ask: &Ask, gap: f32, near: f32) -> bool {
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
    let current = current_target(me, world)?;
    let walls = attacks_walls(me);
    let mut st = Statistics::default();
    let narrowed = add_current(me, ask, current, &mut st, world)?;
    add_extra(me, ask, &mut st, world)?;
    let mut walked = Walked::new(narrowed, walls);
    walked.walk_lists(me, ask, lists, &mut st, world)?;
    if walls && !walked.beyond_walls && walked.gate.0 != -1 {
        return Ok(Choice {
            target: walked.gate.0,
            retries: Vec::new(),
        });
    }
    if st.entries.is_empty() {
        walked.walk_later(me, ask, &mut st, world)?;
        if st.entries.is_empty() {
            return Ok(Choice {
                target: walked.last_resort(),
                retries: Vec::new(),
            });
        }
    }
    let ranked = rank_entries(me, &mut st, world)?;
    choose(me, ask, current, &st, ranked, world, walked.last_resort())
}

/// The current target, when the world holds it alive (state 2 at most) and not allied.
fn current_target(
    me: &Searcher,
    world: &mut dyn Battlefield,
) -> Result<Option<Thing>, crate::GenieError> {
    if me.target == -1 {
        return Ok(None);
    }
    Ok(world
        .thing(me.target)?
        .filter(|t| t.state <= 2 && !t.allied))
}

/// Whether the unit attacks walls: its targets' classes name a wall or a gate, or its owner's
/// flag allows them outside the defend patrol of a unit that is no siege.
const fn attacks_walls(me: &Searcher) -> bool {
    let defending = me.order == DEFEND_PATROL && me.class != SIEGE;
    matches!(me.target_types[0], 27 | 39)
        || matches!(me.target_types[1], 27 | 39)
        || (me.walls_allowed && !defending)
}

/// The current target's entry, when within the unit's range or the unit moves; outside an idle
/// check, one within the reference distance that is no building (3) nor farm (49) narrows the
/// search to its category. The category the search keeps to, −1 for any.
fn add_current(
    me: &Searcher,
    ask: &Ask,
    current: Option<Thing>,
    st: &mut Statistics,
    world: &mut dyn Battlefield,
) -> Result<i32, crate::GenieError> {
    let Some(t) = current.filter(|t| t.distance <= me.range || 0.0 < me.speed) else {
        return Ok(-1);
    };
    let k = category(t.master, t.class);
    st.add(&world.candidate(me.target, t.distance, k)?);
    let narrows = !ask.idle && t.distance <= ask.distance && !matches!(t.class, 3 | 49);
    Ok(if narrows { k } else { -1 })
}

/// The extra objects' entries but the current target's, by the attack check: one under the
/// least range counts at its gap plus the weapon range plus [`ONE`] (none for a unit that does
/// not move).
pub(super) fn add_extra(
    me: &Searcher,
    ask: &Ask,
    st: &mut Statistics,
    world: &mut dyn Battlefield,
) -> Result<(), crate::GenieError> {
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
    Ok(())
}

/// The relation lists' walks (the main one and the retry walk): the category the search keeps
/// to, whether the unit attacks walls, the retarget entries' targets the main walk leaves for
/// the retry walk, the closest gate and wall (id, gap; −1 for none) and whether anything but
/// walls counted.
struct Walked {
    narrowed: i32,
    walls: bool,
    later: Vec<(i32, usize)>,
    gate: (i32, f32),
    wall: (i32, f32),
    beyond_walls: bool,
}

impl Walked {
    /// Before the walks: no gate, no wall, nothing left for later.
    const fn new(narrowed: i32, walls: bool) -> Self {
        Self {
            narrowed,
            walls,
            later: Vec::new(),
            gate: (-1, -1.0),
            wall: (-1, -1.0),
            beyond_walls: false,
        }
    }

    /// The main walk: each list from enemies down to Gaia's (a human player's down to 2 in a
    /// scenario) until the statistics are full, each record but the current target and the
    /// extra objects; a retarget entry's target waits for the retry walk, the others count by
    /// their tests within the sight ([`Walked::counted`]).
    fn walk_lists(
        &mut self,
        me: &Searcher,
        ask: &Ask,
        lists: &[Vec<i32>; 5],
        st: &mut Statistics,
        world: &mut dyn Battlefield,
    ) -> Result<(), crate::GenieError> {
        let lowest = if me.scenario && !me.computer { 2 } else { 0 };
        for kind in (lowest..=4).rev() {
            for &id in &lists[kind] {
                if STATISTICS <= st.entries.len() {
                    return Ok(());
                }
                if id == me.target || ask.extra.contains(&id) {
                    continue;
                }
                if me.retries.contains(&id) {
                    self.later.push((id, kind));
                    continue;
                }
                let Some((t, gap, k)) = self.counted(me, ask, id, kind, me.sight, world)? else {
                    continue;
                };
                match t.class {
                    GATE => {
                        if self.gate.1 < 0.0 || gap < self.gate.1 {
                            self.gate = (id, gap);
                        }
                        if !self.walls {
                            continue;
                        }
                    }
                    WALL => {
                        if !self.note_wall(id, gap) {
                            continue;
                        }
                    }
                    _ => self.beyond_walls = true,
                }
                st.add(&world.candidate(id, gap, k)?);
            }
        }
        Ok(())
    }

    /// The retry walk over the retarget entries' targets the main walk left: the same tests
    /// within the unit's range instead of the sight; a gate counts only for a unit that attacks
    /// walls, and notes nothing.
    fn walk_later(
        &mut self,
        me: &Searcher,
        ask: &Ask,
        st: &mut Statistics,
        world: &mut dyn Battlefield,
    ) -> Result<(), crate::GenieError> {
        for (id, kind) in std::mem::take(&mut self.later) {
            let Some((t, gap, k)) = self.counted(me, ask, id, kind, me.range, world)? else {
                continue;
            };
            let counts = match t.class {
                GATE => self.walls,
                WALL => self.note_wall(id, gap),
                _ => true,
            };
            if counts {
                st.add(&world.candidate(id, gap, k)?);
            }
        }
        Ok(())
    }

    /// The object `id` of list `kind`, its gap and its category when it passes the shared tests
    /// ([`tested`]), the ranges ([`within`], `near` the walk's) and the narrowed category.
    fn counted(
        &self,
        me: &Searcher,
        ask: &Ask,
        id: i32,
        kind: usize,
        near: f32,
        world: &mut dyn Battlefield,
    ) -> Result<Option<(Thing, f32, i32)>, crate::GenieError> {
        let Some(t) = tested(me, ask, id, kind, world)? else {
            return Ok(None);
        };
        let gap = t.distance;
        if !within(me, ask, gap, near) {
            return Ok(None);
        }
        let k = category(t.master, t.class);
        if self.narrowed != -1 && k != self.narrowed {
            return Ok(None);
        }
        Ok(Some((t, gap, k)))
    }

    /// A wall `id` at `gap`: the closest wall when it is the first or nearer; whether it counts
    /// (only for a unit that attacks walls).
    fn note_wall(&mut self, id: i32, gap: f32) -> bool {
        if self.wall.1 < 0.0 || gap < self.wall.1 {
            self.wall = (id, gap);
        }
        self.walls
    }

    /// The answer without a target: the closest wall for a unit that attacks walls, else −1.
    const fn last_resort(&self) -> i32 {
        if self.walls { self.wall.0 } else { -1 }
    }
}

/// The scores ([`Statistics::score`]), the ranks and the sort: a fixed entry but category 6 by
/// its weapon range (3 above the epsilon, else 2, a farm 1); the others 4 for a unit of
/// priority 1 or of 0 but [`PRIORITY_0_MASTERS`], else 0. Whether the unit ranks so.
fn rank_entries(
    me: &Searcher,
    st: &mut Statistics,
    world: &mut dyn Battlefield,
) -> Result<bool, crate::GenieError> {
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
    Ok(ranked)
}

/// The choice over the sorted entries: the current target when it holds
/// ([`current_holds`]); the first three by the quick path (a moving one, or with nothing
/// moving or a unit of another rank rule); more than 5 moving entries outside an idle check:
/// the closest moving one by its path; in an attack work state a current target on top;
/// without path checks the top; else the first three by their paths, each one that fails
/// ignored for 1 s (moving) or 2 s; last `last_resort`.
#[allow(clippy::too_many_arguments)]
fn choose(
    me: &Searcher,
    ask: &Ask,
    current: Option<Thing>,
    st: &Statistics,
    ranked: bool,
    world: &mut dyn Battlefield,
    last_resort: i32,
) -> Result<Choice, crate::GenieError> {
    let range = me.range;
    let mut choice = Choice::default();
    choice.target = if current_holds(me, ask, current, st, world)? {
        me.target
    } else if let Some(id) = quick_pick(st, ranked, range, world)? {
        id
    } else {
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
            me.target
        } else if ask.paths == 0 {
            top
        } else {
            path_pick(me, st, tried, world, &mut choice.retries)?.unwrap_or(last_resort)
        }
    };
    Ok(choice)
}

/// Whether the current target answers: fewer than 5 moving entries and a current target that
/// moves (or nothing moves at all), outside an idle check, when it engages or nothing is armed,
/// by its path (without path checks at once).
fn current_holds(
    me: &Searcher,
    ask: &Ask,
    current: Option<Thing>,
    st: &Statistics,
    world: &mut dyn Battlefield,
) -> Result<bool, crate::GenieError> {
    let Some(t) = current else {
        return Ok(false);
    };
    Ok(st.moving < FEW_MOVING
        && ((t.speed < EPSILON && st.moving == 0) || 0.0 < t.speed)
        && !ask.idle
        && (world.engages(me.target)? || st.armed < 1)
        && (ask.paths == 0 || world.path(me.target, me.range)?))
}

/// The first of the first three entries with a quick path: a moving one, or any with nothing
/// moving or for a unit of another rank rule.
fn quick_pick(
    st: &Statistics,
    ranked: bool,
    range: f32,
    world: &mut dyn Battlefield,
) -> Result<Option<i32>, crate::GenieError> {
    for &i in st.order.iter().take(3) {
        let e = st.entries[i];
        if (e.moving || st.moving < 1 || !ranked) && world.quick_path(e.id, range)? {
            return Ok(Some(e.id));
        }
    }
    Ok(None)
}

/// The first of the first three entries but `tried` with a path; each one before it that
/// fails is ignored for 1 s (moving) or 2 s from now (`retries`, in order).
fn path_pick(
    me: &Searcher,
    st: &Statistics,
    tried: i32,
    world: &mut dyn Battlefield,
    retries: &mut Vec<(i32, u32)>,
) -> Result<Option<i32>, crate::GenieError> {
    for &i in st.order.iter().take(3) {
        let e = st.entries[i];
        if e.id != tried && world.path(e.id, me.range)? {
            return Ok(Some(e.id));
        }
        let wait = if e.moving { RETRY_MOVING } else { RETRY_OTHER };
        retries.push((e.id, me.now.wrapping_add(wait)));
    }
    Ok(None)
}

/// 1 (`DAT_14330eb8c`): an extra object under the least range counts this much beyond the
/// weapon range.
pub(super) const ONE: f32 = crate::math::image::best_target::ONE;
