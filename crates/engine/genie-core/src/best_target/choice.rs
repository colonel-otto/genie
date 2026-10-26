//! `bestUnitToAttack` itself (`FUN_140c44680`): the object tests, the ranges and the choice.

use crate::best_target::{
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

/// The object tests the statistics' main and retry walks share: the world holds it with a unit
/// AI, alive, not allied, not inside another; the unit AI's attack check; a combat unit's
/// friends' scan; a neutral object's test; a Gaia unit others take by sight is passed outside
/// the scenarios; its flags; a computer player's tests; the classes without `any_class`; a
/// computer player's unit's Gaia holdings and boats. The object and its category, or None.
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
pub(super) const ONE: f32 = crate::image::best_target::ONE;
