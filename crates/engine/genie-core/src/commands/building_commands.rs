//! The commands of a building: SET GATHER POINT, the command queue and RESEARCH.

use crate::GenieError;
use crate::commands::{CASTLE_AGE, CASTLE_AGE_MODE, GatherPoint, ResearchAt};
use crate::world::{BLOCKS_RESEARCH, BUILDING_KIND, RESEARCH_ACTION, World};

impl World {
    /// SET GATHER POINT's handler ([`Command::GatherPoint`]).
    ///
    /// [`Command::GatherPoint`]: crate::commands::Command::GatherPoint
    pub(crate) fn gather_point(
        &mut self,
        player: usize,
        ids: &[i32],
        [x, y]: [f32; 2],
        (target, target_master): (i32, i16),
        own: bool,
    ) -> Result<(), GenieError> {
        for &id in ids {
            // another player's object (the game clears its id) or one that is gone
            let Some(i) = self.players[player].objects.iter().position(|u| u.id == id) else {
                continue;
            };
            let unit = &self.players[player].objects[i];
            let master = self
                .masters
                .get(player)
                .and_then(|m| m.get(unit.life.master_id))
                .ok_or(GenieError::unsupported(
                    "SET GATHER POINT on a master the slice does not have",
                ))?;
            // FUN_141625a20's count with no object, for the object's own player
            if master.garrison == 0 && self.trainable_count(player, master.id) <= 0 {
                continue;
            }
            let z = unit.gather_point.map_or(0.0, |g| g.point[2]);
            let gather = if own {
                Some(GatherPoint {
                    point: [unit.position[0], unit.position[1], z],
                    target: unit.id,
                    target_master: unit.life.master_id,
                })
            } else if target == -1 && x == -1.0 && y == -1.0 {
                None
            } else {
                Some(GatherPoint {
                    point: [x, y, z],
                    target,
                    target_master,
                })
            };
            let unit = &mut self.players[player].objects[i];
            unit.gather_point = gather;
            // the unit AI's slot 0x210 (FUN_140c16790): its command queue empties
            if let Some(ai) = unit.ai.as_mut() {
                ai.building_orders.clear();
            }
        }
        Ok(())
    }

    /// `BUILDING_COMMAND_QUEUE`'s handler (`0x140e64310`) for `player`: each object of `ids`
    /// the world holds with a unit AI queues the step (its slot 0x208, `FUN_140c15740`: the
    /// player, the order `0x2db` on a target or `0x2c6` on the point, priority −1, the target,
    /// its owner and class, the point with height −1, range 0, not ready). A building first puts
    /// its gather point in the queue when it has one, the queue is empty (slot 0x218) and the
    /// point is not on itself, then the gather point clears (`FUN_1411c7ff0` with −1). A target
    /// the world does not hold queues the point alone. The view's marker (`FUN_1414cd6d0`) is not
    /// held.
    pub(crate) fn command_queue(
        &mut self,
        player: usize,
        ids: &[i32],
        target: i32,
        [x, y]: [f32; 2],
    ) -> Result<(), GenieError> {
        let step = |turns: &Self,
                    t: i32,
                    at: [f32; 2]|
         -> Result<crate::unit_ai::QueuedOrder, GenieError> {
            let named = if t == -1 { None } else { turns.object(t) };
            let (target, owner, class) = match named {
                Some((tp, u)) => (
                    t,
                    tp as i32,
                    i32::from(turns.master_of(tp, u.life.master_id)?.class),
                ),
                None => (-1, -1, -1),
            };
            Ok(crate::unit_ai::QueuedOrder {
                issuer: player as i32,
                order: if target < 0 {
                    crate::unit_ai::MOVE_ORDER
                } else {
                    crate::unit_ai::ORDER_ON_TARGET
                },
                priority: -1,
                target,
                target_owner: owner,
                point: [at[0], at[1], -1.0],
                range: 0.0,
                shown: [-1.0, -1.0],
                ready: false,
                target_type: class,
            })
        };
        for &id in ids {
            let Some(crate::orders::Place::Live(p, i)) = self.place(id) else {
                if self.place(id).is_some() {
                    return Err(GenieError::unsupported(
                        "a command queue on a sleeping object",
                    ));
                }
                continue;
            };
            let unit = &self.players[p].objects[i];
            let Some(ai) = unit.ai.as_ref() else {
                continue;
            };
            let building = self.master_of(p, unit.life.master_id)?.kind == crate::orders::BUILDING;
            let mut queued = Vec::new();
            if building
                && let Some(g) = unit.gather_point
                && ai.building_orders.is_empty()
                && g.target != unit.id
            {
                // the gather point's own target id stays even when the world lost it
                let mut first = step(self, g.target, [g.point[0], g.point[1]])?;
                if first.target == -1 && g.target != -1 {
                    first.target = g.target;
                    first.order = crate::unit_ai::ORDER_ON_TARGET;
                }
                queued.push(first);
            }
            queued.push(step(self, target, [x, y])?);
            let unit = &mut self.players[p].objects[i];
            if building {
                unit.gather_point = None;
            }
            if let Some(ai) = unit.ai.as_mut() {
                ai.building_orders.extend(queued);
            }
        }
        Ok(())
    }

    /// `do_command`'s case 0x65 (`FUN_140e6f470`), RESEARCH's check before its handler: the
    /// count of buildings it lists (`+0x8` as a short) below 1 is refused; the packet's building
    /// (`+4`), when the world holds it, must be the player's own, or any while players may order
    /// each other's (the setting `+0x1074`), and listed alone it must be held; with more listed,
    /// the listed ones (`+0x11..`) as the unit commands' ([`World::command_check`]). None for a
    /// refused one, else the list with −1 for the ones left out. The packet's size tests are the
    /// decoder's, and the world's `+0xb12` cheat is not held.
    pub(crate) fn research_check(&self, player: usize, r: &ResearchAt) -> Option<Vec<i32>> {
        let listed = usize::try_from(r.count).unwrap_or(0);
        if listed == 0 {
            return None;
        }
        let own = self
            .object(r.building)
            .map(|(p, _)| p == player || self.settings.cheats_enabled);
        match own {
            Some(false) => return None,
            None if listed == 1 => return None,
            _ => {}
        }
        if listed == 1 {
            return Some(r.buildings.clone());
        }
        let n = listed.min(r.buildings.len());
        let mut checked = self.command_check(player, &r.buildings[..n])?;
        checked.extend_from_slice(&r.buildings[n..]);
        Some(checked)
    }

    /// RESEARCH (`0x140e7e270`) for `player`: a target other than −1, or a technology the tree
    /// has not got, does nothing. The costs it lists (`FUN_1413d4a00`) come off the pending
    /// debit (`FUN_141630d10`). The building is the packet's (the least loaded of several,
    /// `FUN_1411d0e90`, and the shared kind's every building, `FUN_1411c3fd0`, are not modelled);
    /// a building of master type 80 alive (state 2) whose master is among the technology's
    /// locations, with the technology in state 1 or repeatable, and without a top action of
    /// 0x7b to 0x7d (`FUN_141227a00`), queues it ([`World::queue_research`]); queued, its unit AI
    /// takes the action value 0x26c. A technology of `linkedTechs.json` ([`World::linked_techs`],
    /// the game settings' table `FUN_140f39f40` looks up) is not modelled (`FUN_140f3a0f0`,
    /// `FUN_140f39e80`). Not held: the interface's view and selection (`FUN_140e6de20`), the
    /// local player's statistics flag. Not modelled: the computer players' parts (`player+0x7a8`, `FUN_1411d5520`), the player's
    /// `+0x648` (0 for every player probed), Castle Age queued in game mode 16 (the world's treaty
    /// monitor's `+0x17`; no other game mode marks it).
    pub(crate) fn research(&mut self, player: usize, r: &ResearchAt) -> Result<(), GenieError> {
        let owner = &self.players[player];
        if r.target != -1 || r.tech < 0 || owner.techs.len() <= r.tech as usize {
            return Ok(());
        }
        let tech = r.tech as usize;
        let def = self.techs.get(tech).ok_or(GenieError::unsupported(
            "a technology the data file has not got",
        ))?;
        let get = |i: i16| crate::technology::Attributes::get(owner, i);
        let factors = crate::technology::factors(r.tech, &get)?;
        let listed = owner.techs[tech].listed_costs(def, factors);
        let mut sums = [0i16; 4];
        for (attribute, amount) in listed {
            if let Some(k) = usize::try_from(attribute).ok().filter(|&k| k < 4) {
                sums[k] = sums[k].wrapping_add(amount);
            }
        }
        for (debit, amount) in self.players[player].pending_debit.iter_mut().zip(sums) {
            *debit -= f32::from(amount);
        }
        let owner = &self.players[player];
        let state = &owner.techs[tech];
        let repeatable = state.repeatable(def, false);
        if def.kind == crate::technology::SHARED_KIND {
            return Err(GenieError::unsupported(
                "RESEARCH of the shared kind (FUN_1411c3fd0 over the packet's buildings)",
            ));
        }
        let building_id = self.least_loaded(player, r);
        if self.linked_techs.contains_key(&r.tech) {
            return Err(GenieError::unsupported(
                "RESEARCH of a linked technology (FUN_140f3a0f0, FUN_140f39e80)",
            ));
        }
        // a technology that is not available (state 1) and may not be queued again is
        // researchable only when it is linked (`FUN_140f39e80`): no building takes it
        if state.state != 1 && !repeatable {
            return Ok(());
        }
        let Some(i) = owner.objects.iter().position(|u| u.id == building_id) else {
            let elsewhere = self.players.iter().any(|q| {
                q.objects
                    .iter()
                    .chain(&q.sleeping)
                    .any(|u| u.id == building_id)
            });
            return if elsewhere {
                Err(GenieError::unsupported(
                    "RESEARCH at another player's object",
                ))
            } else {
                Ok(())
            };
        };
        let building = &owner.objects[i];
        let master = self.master_of(player, building.life.master_id)?;
        let at = state
            .locations
            .iter()
            .any(|l| l.building == building.life.master_id);
        if master.kind != BUILDING_KIND || building.life.state != 2 || !at {
            return Ok(());
        }
        if owner.computer {
            return Err(GenieError::unsupported(
                "a computer player's RESEARCH (FUN_1411d5520, player+0x7a8)",
            ));
        }
        let top = building.top_action_kind().unwrap_or(0);
        if building.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "RESEARCH at a building with an unmodelled action",
            ));
        }
        if BLOCKS_RESEARCH.contains(&top) {
            return Ok(());
        }
        if self.queue_research(player, i, r.tech, r.shift)? {
            // Castle Age queued in game mode 16 marks the world's treaty monitor (`+0x820`, its
            // `+0x17`), which the slice does not hold; any other game mode leaves it
            if r.tech == CASTLE_AGE && self.settings.game_mode == CASTLE_AGE_MODE {
                return Err(GenieError::unsupported(
                    "RESEARCH of Castle Age in game mode 16 (the treaty monitor's +0x17)",
                ));
            }
            let building = &mut self.players[player].objects[i];
            if let Some(ai) = building.ai.as_mut() {
                ai.set_action(RESEARCH_ACTION);
            }
        }
        Ok(())
    }

    /// `FUN_1411d70f0` for the building at `i` of `player`: one research, five with `shift`
    /// unless the technology allows only one at a time or its effect upgrades a unit, queued by
    /// the building's part with no master and no button ([`World::queue_at`]). True when any
    /// was queued. The game settings' instant technologies (`FUN_140f39f40`'s `+0x20`, which
    /// `FUN_1413cdcd0` researches at once) are linked ones, which RESEARCH refuses before.
    pub(crate) fn queue_research(
        &mut self,
        player: usize,
        i: usize,
        tech: i16,
        shift: bool,
    ) -> Result<bool, GenieError> {
        let t = usize::try_from(tech).expect("RESEARCH queues a technology of the tree");
        let def = &self.techs[t];
        let state = &self.players[player].techs[t];
        let one = !state.allow_multiple || def.upgrades();
        let count: u16 = if !shift || one { 1 } else { 5 };
        self.queue_at(player, i, -1, count, tech, -1)
    }
}
