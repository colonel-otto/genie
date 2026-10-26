//! The gates: the units in a gate, a player's gate list and passes, and the gates opened before
//! and closed after a player's update.

use crate::GenieError;
use crate::map::spatial::Spatial;
use crate::turn::{GATE_COUNT_CAP, GATE_EPSILON, GATE_GROW, GATE_QUERY_RECORDS, Turns};

/// The units in gate `gate` (`FUN_1411d0570`): the obstruction manager's query from its first
/// annex to its second (`+0xbc0`, `+0xbc8`) with flags 0x10, the gate and its annexes ignored
/// (`FUN_1410562e0`), every record and the query grown by [`GATE_GROW`] (`FUN_1410562d0`), any
/// kind and player ([`Spatial::walk_hits`]; its list holds 10 records, more are not modelled).
/// Of the objects it lists, those not of the classes 27, 39 and 52, nor of class 3 standing
/// still, and either moving (a top speed of at least [`GATE_EPSILON`]) or not Gaia's count, at
/// most 25; the second answer is how many of them are of a player the gate's counts an ally
/// (`FUN_141663d90` with the ally flag).
pub(crate) fn gate_units(
    spatial: &Spatial,
    gate: i32,
    annexes: &[i32],
) -> Result<(Vec<i32>, usize), GenieError> {
    let [a, b] = annexes[..] else {
        return Err(GenieError::unsupported("a gate without its two annexes"));
    };
    let place = |id| {
        spatial
            .get(id)
            .map(|p| [p.position[0], p.position[1]])
            .ok_or(GenieError::unsupported("a gate's annex without a place"))
    };
    let (from, to) = (place(a)?, place(b)?);
    let rect = [
        from[0].min(to[0]),
        from[1].min(to[1]),
        from[0].max(to[0]),
        from[1].max(to[1]),
    ];
    let owner = spatial
        .get(gate)
        .map(|p| p.player)
        .ok_or(GenieError::unsupported("a gate without a place"))?;
    spatial.check_known_records([
        rect[0] - GATE_GROW,
        rect[1] - GATE_GROW,
        rect[2] + GATE_GROW,
        rect[3] + GATE_GROW,
    ])?;
    let hits = spatial.walk_hits(from, to, rect, GATE_GROW, 1.0, Some(owner), &[gate, a, b])?;
    if GATE_QUERY_RECORDS < hits.len() {
        return Err(GenieError::unsupported(
            "a gate's query of more than 10 records",
        ));
    }
    let (mut units, mut allied) = (Vec::new(), 0);
    for id in hits {
        if GATE_COUNT_CAP <= units.len() {
            break;
        }
        let Some(p) = spatial.get(id) else {
            continue;
        };
        let counts = !matches!(p.class, 27 | 39 | 52)
            && (p.class != 3 || p.speed != 0.0)
            && (GATE_EPSILON <= p.speed || p.player != 0);
        if counts {
            units.push(id);
            if spatial.allied_with(owner, p.player) {
                allied += 1;
            }
        }
    }
    Ok((units, allied))
}

impl Turns {
    /// Player `q`'s gate list (`+0x1360`, `FUN_141616200` adds to its front): its live objects
    /// of class 39 whose master transforms ([`crate::masters::BuildingMaster::transform_unit`]),
    /// newest first.
    pub(crate) fn gate_list(&self, q: usize) -> Vec<usize> {
        let masters = self.masters.get(q);
        let owner = &self.players[q];
        (0..owner.objects.len())
            .rev()
            .filter(|&i| {
                masters
                    .and_then(|m| m.get(owner.objects[i].life.master_id))
                    .is_some_and(|m| {
                        m.class == crate::map::spatial::GATE_CLASS
                            && m.building.as_ref().is_some_and(|b| 0 <= b.transform_unit)
                    })
            })
            .collect()
    }

    /// A gate's slot 0x678 (`FUN_1411d4a30`): player `p` may pass object `i` of player `q` when it
    /// stands (state below 3), its master is of class 39 and transforms (`+0x9ba` not -1), it is
    /// not locked (`mGateLocked`) and its owner and `p` count each other as allies
    /// (`FUN_141663dc0`, `FUN_141663d90`; a player counts itself).
    pub(crate) fn gate_passes(&self, q: usize, i: usize, p: usize) -> bool {
        let gate = &self.players[q].objects[i];
        let transforming = self
            .masters
            .get(q)
            .and_then(|m| m.get(gate.life.master_id))
            .is_some_and(|m| {
                m.class == crate::map::spatial::GATE_CLASS
                    && m.building.as_ref().is_some_and(|b| b.transform_unit != -1)
            });
        gate.life.state < 3
            && transforming
            && !gate.gate.is_some_and(|g| g.locked)
            && self.spatial.allied_with(q, p)
            && self.spatial.allied_with(p, q)
    }

    /// `BaseWorld::update` before player `p` updates (not Gaia): the gates of each player `q`
    /// that is `p` or counts `p` an ally and is counted one by it, unless `q`'s are open already
    /// (`open`, the update's flags), open for `p` (`q`'s slot 0x318, `FUN_14162faf0`): each gate
    /// `p` may pass ([`Turns::gate_passes`]) loses its record, its tiles' marks and its mesh shape
    /// (`FUN_141055b40`), so `p`'s units walk and plan through it.
    pub(crate) fn open_gates(&mut self, p: usize, open: &mut [bool]) -> Result<(), GenieError> {
        if p == 0 {
            return Ok(());
        }
        let players = self.players.len();
        for (q, is_open) in open.iter_mut().enumerate().take(players).skip(1) {
            if *is_open
                || (q != p && !(self.spatial.allied_with(p, q) && self.spatial.allied_with(q, p)))
            {
                continue;
            }
            for i in self.gate_list(q) {
                if !self.gate_passes(q, i, p) {
                    continue;
                }
                let id = self.players[q].objects[i].id;
                if self
                    .spatial
                    .get(id)
                    .is_some_and(|g| g.obstruction.is_some_and(|o| o.kind == 10))
                {
                    return Err(GenieError::unsupported(
                        "a gate of obstruction kind 10 opens (FUN_140f65580)",
                    ));
                }
                self.spatial.drop_record(id);
                self.pathing.removed(id);
            }
            *is_open = true;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        Ok(())
    }

    /// `BaseWorld::update` after player `p` updated (not Gaia): each open player `q`'s gates close
    /// (`q`'s slot 0x320, `FUN_141619480`) unless the next player counts `q` an ally and is
    /// counted one by it (they stay open for it): each gate `p` may pass gets its record, its
    /// tiles' marks and its mesh shape again (`FUN_141056460`), but for a part of obstruction
    /// kind 0 or 12 or of class 0 ([`crate::map::spatial::Obstruction::recorded`]).
    pub(crate) fn close_gates(&mut self, p: usize, open: &mut [bool]) {
        if p == 0 {
            return;
        }
        let (next, players) = (p + 1, self.players.len());
        for (q, is_open) in open.iter_mut().enumerate().take(players).skip(1) {
            if !*is_open {
                continue;
            }
            if next < players
                && self.spatial.allied_with(next, q)
                && self.spatial.allied_with(q, next)
            {
                continue;
            }
            for i in self.gate_list(q) {
                if !self.gate_passes(q, i, p) {
                    continue;
                }
                let gate = &self.players[q].objects[i];
                let (id, master) = (gate.id, gate.life.master_id);
                let Some(m) = self.masters.get(q).and_then(|t| t.get(master)) else {
                    continue;
                };
                let record = crate::map::spatial::Obstruction {
                    kind: m.obstruction_type,
                    class: m.obstruction_class,
                };
                // a gate part of obstruction kind 0 (the gate's middle, master 91) gets none
                if record.recorded()
                    && self
                        .spatial
                        .get(id)
                        .is_some_and(|g| g.obstruction.is_none())
                {
                    self.spatial.add_record(id, record);
                    if let Some(placed) = self.spatial.get(id).copied() {
                        self.pathing.place_shape(id, &placed);
                    }
                }
            }
            *is_open = false;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
    }
}
