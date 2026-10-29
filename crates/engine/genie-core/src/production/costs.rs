//! A queue entry's costs: the test and the payment of a unit's or a technology's costs, and
//! their refunds when an entry is cancelled (`FUN_141630530` and the cancels' calls).

use crate::GenieError;
use crate::objects::masters::master_type::BUILDING;
use crate::objects::masters::master_type::COMBAT;
use crate::objects::masters::{Master, Masters};
use crate::production::{HEADROOM, POPULATION, POPULATION_CAP, QUEUED};

/// `FUN_141630530`: a queued unit's refund. For a master of type 70 and up each of its cost
/// entries with an attribute and a paid flag gives back what the queue entry's `costs` paid
/// of that attribute (the last pair that names it; 0 without one), added straight into the
/// player's attribute (no side effects).
pub fn refund(
    master: &Master,
    costs: &[(i32, f32); 3],
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<(), GenieError> {
    if master.kind < COMBAT {
        return Ok(());
    }
    for &(attribute, _, paid) in master.cost.iter().take(3) {
        if attribute < 0 || !paid {
            continue;
        }
        let back = costs
            .iter()
            .rev()
            .find(|c| c.0 != -1 && c.0 == i32::from(attribute))
            .map_or(0.0, |c| c.1);
        let held = attributes.get(attribute)?;
        attributes.set(attribute, held + back)?;
    }
    Ok(())
}

/// `FUN_1413d15e0`'s refund of `count` researches of a queue entry that paid `costs`: each
/// pair with an attribute gives back the count times its amount, straight into the player's
/// attribute. (Its technology's state 1 and work done 0 are the caller's.)
pub fn refund_research(
    costs: &[(i32, f32); 3],
    count: i16,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<(), GenieError> {
    for &(attribute, amount) in costs {
        // the entry's attributes are the costs' i16 attributes (FUN_1411c3190, the load)
        let Ok(attribute) = i16::try_from(attribute) else {
            continue;
        };
        if attribute < 0 {
            continue;
        }
        let held = attributes.get(attribute)?;
        attributes.set(attribute, held + f32::from(count) * amount)?;
    }
    Ok(())
}

/// `FUN_141630770` with `n` 1, the make action's destructor's refund of a unit its queue did
/// not pay for: each cost entry of a master of type 70 and up with an attribute and a paid
/// flag gives back its amount times `n` (an integer product), straight into the attribute.
pub fn refund_master(
    master: &Master,
    n: i32,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<(), GenieError> {
    if master.kind < COMBAT {
        return Ok(());
    }
    for &(attribute, amount, paid) in master.cost.iter().take(3) {
        if attribute < 0 || !paid {
            continue;
        }
        let held = attributes.get(attribute)?;
        attributes.set(attribute, (i32::from(amount) * n) as f32 + held)?;
    }
    Ok(())
}

/// The research cost modifier (attribute 85, `+0x2a8`), which scales a refunded technology's
/// costs above 0.
pub(crate) const RESEARCH_COST_MODIFIER: i16 = 85;

/// `FUN_1413ccc00` with `param_3` 1 for technology `tech`: in state 2 or 4 it gives back its
/// `.dat` costs (each entry with an attribute and a paid flag: its amount plus the state's
/// adjustment, which the research cost modifier, attribute 85 above 0 and not for an age,
/// scales truncated to 16 bits), straight into the attributes, and is in state 1 with no work
/// done. True when it was; in another state nothing happens.
pub fn refund_tech(
    state: &mut crate::technology::TechState,
    def: &crate::technology::TechDef,
    tech: i16,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<bool, GenieError> {
    use crate::math::conversions::cvttss2si;
    if !matches!(state.state, 2 | 4) {
        return Ok(false);
    }
    let modifier = if crate::technology::is_age(tech) {
        0.0
    } else {
        attributes.get(RESEARCH_COST_MODIFIER)?
    };
    for (j, c) in def.costs.iter().enumerate() {
        if c.attribute < 0 || c.paid == 0 {
            continue;
        }
        let mut v = (i32::from(state.cost_adj[j]) + i32::from(c.amount)) as f32;
        if 0.0 < modifier {
            v = f32::from(cvttss2si(v * modifier) as i16);
        }
        let held = attributes.get(c.attribute)?;
        attributes.set(c.attribute, held + v)?;
    }
    state.state = 1;
    state.research_done = 0.0;
    Ok(true)
}

/// The cost multiplier `FUN_14162ef60`'s callers pass (`DAT_14330eb8c`, 1.0).
const ONE: f32 = 1.0;

/// A master's three cost entries `(attribute, amount, paid)`, −1 for an entry it lacks.
fn cost_entry(master: &Master, j: usize) -> (i16, i16, bool) {
    master.cost.get(j).copied().unwrap_or((-1, 0, false))
}

/// The master whose costs the cost tests read for player's master `m` (`FUN_141618db0`,
/// `FUN_141614450`): a building (type 80) that is an annex (a head unit, `+0x9b8`, the player
/// has) reads its head's, and the head's stack unit's (`+0x986`) when the player has that one,
/// then only the paid entries (the second value true).
#[must_use]
pub fn costed<'m>(masters: &'m Masters, m: &'m Master) -> (&'m Master, bool) {
    if m.kind != BUILDING {
        return (m, false);
    }
    let Some(head) = m
        .building
        .as_ref()
        .filter(|b| b.head_unit != -1)
        .and_then(|b| masters.get(b.head_unit))
    else {
        return (m, false);
    };
    head.building
        .as_ref()
        .filter(|b| b.stack_unit != -1)
        .and_then(|b| masters.get(b.stack_unit))
        .map_or((head, false), |stack| (stack, true))
}

/// `FUN_141618db0` without a reason out and without the pending debits (`param_6`, `param_7`
/// 0): the player can pay `count` units of its master `master`. A master the player lacks or
/// of a type below 70 cannot. Each cost entry of the costed master ([`costed`]) with an
/// attribute (and paid, for an annex's) needs its amount times `count` at most what the
/// attribute holds (`COMISS`, `JA`: a NaN passes); the population's entry (attribute 4) is
/// tested only with `population` (`param_5` 0): the population (attribute 11) below the cap
/// (32), and the queued units (80) plus the population plus the need at most the cap or the
/// headroom (4) plus the population, the smaller.
pub fn cost_test(
    masters: &Masters,
    master: i16,
    count: f32,
    population: bool,
    attributes: &dyn crate::technology::Attributes,
) -> Result<bool, GenieError> {
    let Some(m) = masters.get(master).filter(|m| COMBAT <= m.kind) else {
        return Ok(false);
    };
    let (m, annex) = costed(masters, m);
    for j in 0..3 {
        let (attribute, amount, paid) = cost_entry(m, j);
        if attribute < 0 || (annex && !paid) {
            continue;
        }
        let need = f32::from(amount) * count;
        if attribute == HEADROOM {
            if !population {
                continue;
            }
            let pop = attributes.get(POPULATION as i16)?;
            let cap = attributes.get(POPULATION_CAP as i16)?;
            if cap <= pop {
                return Ok(false);
            }
            let room = attributes.get(HEADROOM)? + pop;
            // CMOVBE: the cap unless it is above the headroom's sum (a NaN takes the cap)
            let limit = if crate::math::fmath::at_most_or_unordered(cap, room) {
                cap
            } else {
                room
            };
            if limit < attributes.get(QUEUED)? + pop + need {
                return Ok(false);
            }
        } else if attributes.get(attribute)? < need {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `FUN_14162fb60`: when the player can pay `count` units of master `master` without the
/// population's test ([`cost_test`] with `param_5` 1), each of the master's own cost entries
/// with an attribute and a paid flag comes off its attribute, its amount times `count`
/// (straight into the attribute; the statistics `FUN_14097daf0` are not the simulation's).
pub fn pay(
    masters: &Masters,
    master: i16,
    count: f32,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<bool, GenieError> {
    if !cost_test(masters, master, count, false, &*attributes)? {
        return Ok(false);
    }
    // the cost test found the master
    let Some(m) = masters.get(master) else {
        return Ok(false);
    };
    for j in 0..3 {
        let (attribute, amount, paid) = cost_entry(m, j);
        if attribute < 0 || !paid {
            continue;
        }
        attributes.subtract(attribute, f32::from(amount) * count)?;
    }
    Ok(true)
}

/// `FUN_14162ef60` (no type out, the multiplier 1.0): master `master`'s three costs as a queue
/// entry records them. Each entry with an attribute takes its amount (times 1.0, truncated to
/// 16 bits); when the player could pay one more ([`cost_test`] with the population) yet holds
/// less of an attribute than its amount, the entry takes what it holds (truncated) and the
/// difference goes to gold: to the first entry of no attribute or of gold (attribute 3, its
/// amount grown; the third takes the difference alone). An amount below 1 leaves no attribute.
/// A master the player lacks or of a type below 70 lists none.
pub fn master_costs(
    masters: &Masters,
    master: i16,
    attributes: &dyn crate::technology::Attributes,
) -> Result<[(i16, i16); 3], GenieError> {
    use crate::math::conversions::cvttss2si;
    let mut out = [(-1i16, 0i16); 3];
    let can = cost_test(masters, master, ONE, true, attributes)?;
    let Some(m) = masters.get(master).filter(|m| COMBAT <= m.kind) else {
        return Ok(out);
    };
    let mut extra = 0i16;
    for (j, slot) in out.iter_mut().enumerate() {
        let (attribute, amount, _) = cost_entry(m, j);
        slot.0 = attribute;
        if attribute < 0 {
            continue;
        }
        slot.1 = cvttss2si(f32::from(amount) * ONE) as i16;
        let held = attributes.get(attribute)?;
        if held < f32::from(slot.1) && can {
            let have = cvttss2si(held) as i16;
            extra = extra.wrapping_add(slot.1.wrapping_sub(have));
            slot.1 = have;
        }
    }
    if 0 < extra {
        let gold = |a: i16| matches!(a, -1 | 3);
        if gold(out[0].0) {
            out[0] = (3, out[0].1.wrapping_add(extra));
        } else if gold(out[1].0) {
            out[1] = (3, out[1].1.wrapping_add(extra));
        } else if gold(out[2].0) {
            out[2] = (3, extra);
        }
    }
    for slot in &mut out {
        if slot.1 < 1 {
            slot.0 = -1;
        }
    }
    Ok(out)
}

/// The costs a queue entry records from listed ones (`FUN_1411c3190`, `FUN_1411c81a0`): each
/// pair's attribute and its amount as a float, added to the slot's cleared 0.
#[must_use]
pub fn as_entry_costs(listed: [(i16, i16); 3]) -> [(i32, f32); 3] {
    listed.map(|(a, n)| (i32::from(a), f32::from(n) + 0.0))
}
