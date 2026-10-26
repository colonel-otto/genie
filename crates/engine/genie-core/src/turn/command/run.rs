//! A command's run in the command phase ([`Turns::run`]), a human player's order and the
//! order units' reset.

use crate::math::rng::Stream;
use crate::turn::Turns;
use crate::turn::command::{Command, Order, QueueAt, ResearchAt};
use crate::unit_ai::GenieError;

impl Turns {
    /// Run a command in the command phase. Where the slice does not model what the command does to
    /// the issuing player's units, that player's lane stops.
    pub(in crate::turn) fn run(&mut self, command: &Command) {
        match *command {
            Command::Build {
                player,
                master,
                ref unit_ai,
                position,
                ref builders,
                queued,
                commits,
                frame,
            } => {
                // TRIBE_Command::do_command's case 0x66 checks the builders first: another
                // player's becomes -1, and a human's BUILD left without one is refused
                let Some(builders) = self.command_check(player, builders) else {
                    return;
                };
                let builders = &builders;
                let master = self.farm_on_rice_terrain(master, position);
                // the pending debit first (FUN_141630d10 with 0), then the builders' test, the
                // placement test and the payment's: a refused BUILD places nothing
                if let Err(e) = self.debit_listed(player, master) {
                    self.stop_player(player, e);
                    return;
                }
                match self.build_has_builder(player, master, builders) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        self.stop_player(player, e);
                        return;
                    }
                }
                match self.build_allowed(player, master, [position[0], position[1]]) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        self.stop_player(player, e);
                        return;
                    }
                }
                let built = self
                    .found(player, master, unit_ai.clone(), position)
                    .and_then(|(id, position)| {
                        // the foundation keeps the command's frame (+0xc54) before its builders
                        if let Some((_, u)) = self.object_mut(id) {
                            u.build_frame = frame;
                        }
                        self.task_builders(
                            player,
                            id,
                            master,
                            position,
                            builders,
                            (queued, commits),
                        )
                    });
                if let Err(e) = built {
                    self.stop_player(player, e);
                }
            }
            Command::Queue {
                player,
                ref buildings,
                unit,
                amount,
                button,
            } => {
                let queue = QueueAt {
                    buildings: buildings.clone(),
                    unit,
                    amount,
                    button,
                };
                if let Err(e) = self.queue(player, &queue) {
                    self.stop_player(player, e);
                }
            }
            Command::Research {
                player,
                building,
                count,
                ref buildings,
                tech,
                target,
                shift,
            } => {
                let r = ResearchAt {
                    building,
                    count,
                    buildings: buildings.clone(),
                    tech,
                    target,
                    shift,
                };
                // do_command's case 0x65 checks the buildings first
                let Some(buildings) = self.research_check(player, &r) else {
                    return;
                };
                let r = ResearchAt { buildings, ..r };
                if let Err(e) = self.research(player, &r) {
                    self.stop_player(player, e);
                }
            }
            Command::Unmodelled { player, ref why } => {
                self.stop_player(player, why.clone());
            }
            Command::Game(ref g) => {
                // what the slice cannot model stops the player the sub-command is for
                if let Err(e) = self.game(g) {
                    let p = g.target(self.players.len()).unwrap_or(g.issuer);
                    self.stop_player(p, e);
                }
            }
            Command::Trade {
                player,
                attribute,
                times,
                buy,
            } => {
                if buy {
                    self.buy(player, attribute, times);
                } else {
                    self.sell(player, attribute, times);
                }
            }
            Command::Gate { player, id } => {
                if let Err(e) = self.gate(player, id) {
                    self.stop_player(player, e);
                }
            }
            Command::CommandQueue {
                player,
                ref ids,
                target,
                point,
            } => {
                if let Err(e) = self.command_queue(player, ids, target, point) {
                    self.stop_player(player, e);
                }
            }
            Command::BuildWall(ref w) => {
                // do_command's case 0x69 checks the builders as BUILD's
                let Some(builders) = self.command_check(w.player, &w.builders) else {
                    return;
                };
                let w = crate::wall::Wall {
                    builders,
                    ..w.clone()
                };
                if let Err(e) = self.build_wall(&w) {
                    self.stop_player(w.player, e);
                }
            }
            Command::Delete { player, id } => {
                if let Err(e) = self.delete(player, id) {
                    self.stop_player(player, e);
                }
            }
            Command::GoBackToWork { player, ref ids } => {
                if let Err(e) = self.go_back_to_work(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::AllBackToWork { player, id } => {
                if let Err(e) = self.all_back_to_work(player, id) {
                    self.stop_player(player, e);
                }
            }
            Command::TownBell {
                player,
                id,
                activate,
            } => {
                if let Err(e) = self.town_bell(player, id, activate) {
                    self.stop_player(player, e);
                }
            }
            Command::Flare {
                player,
                target,
                point,
                players,
            } => {
                if let Err(e) = self.flare(target, point, players) {
                    self.stop_player(player, e);
                }
            }
            Command::Tribute {
                player,
                to,
                ref gifts,
            } => {
                if let Err(e) = self.tribute(player, to, gifts) {
                    self.stop_player(player, e);
                }
            }
            Command::Unload {
                player,
                ref ids,
                point,
                flag,
                unit_type,
                sound,
            } => {
                if let Err(e) = self.unload(player, ids, point, flag, unit_type, sound) {
                    self.stop_player(player, e);
                }
            }
            Command::UnitOrder {
                player,
                ref ids,
                target,
                through_ai,
                nearby,
                action,
                param,
                human,
            } => {
                if human {
                    self.human_order(ids);
                }
                let result = match action {
                    5 => self.garrison_order(player, ids, target, through_ai, nearby),
                    4 => self.cancel_order(ids, param, through_ai),
                    _ => self.unit_order(player, ids, action),
                };
                if let Err(e) = result {
                    self.stop_player(player, e);
                }
            }
            Command::TransformObject { player, id, master } => {
                if let Err(e) = self.transform_object(player, id, master) {
                    self.stop_player(player, e);
                }
            }
            Command::Resign { player, dropped } => {
                if let Err(e) = self.resign(player, dropped) {
                    self.stop_player(player, e);
                }
            }
            Command::AutoScout { player, ref ids } => {
                if let Err(e) = self.auto_scout(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::Stop { player, ref ids } => {
                if let Err(e) = self.stop_command(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::Patrol {
                player,
                ref ids,
                ref waypoints,
                reset,
            } => {
                if let Err(e) = self.patrol(player, ids, waypoints, reset) {
                    self.stop_player(player, e);
                }
            }
            Command::AttackMove {
                player,
                ref ids,
                ref waypoints,
                reset,
                queued,
                ignore_pending,
            } => {
                let sweep = crate::orders::Sweep {
                    patrol: false,
                    reset,
                    queued,
                    ignore_pending,
                };
                if let Err(e) = self.attack_move(player, ids, waypoints, sweep) {
                    self.stop_player(player, e);
                }
            }
            Command::SeekShelter { player, ref ids } => {
                if let Err(e) = self.seek_shelter(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::UnitAiState {
                player,
                ref ids,
                state,
            } => {
                if let Err(e) = self.unit_ai_state(player, ids, state) {
                    self.stop_player(player, e);
                }
            }
            Command::GatherPoint {
                player,
                ref ids,
                point,
                target,
                target_master,
                own,
            } => {
                if let Err(e) = self.gather_point(player, ids, point, (target, target_master), own)
                {
                    self.stop_player(player, e);
                }
            }
            Command::Order {
                player,
                ref units,
                order,
            } => {
                // the command's check (MOVE's FUN_140e553f0, INTERACT's FUN_140e55250): a unit the
                // world does not hold, or of another player without the setting +0x1074, becomes
                // -1; a human player's command left without a unit does nothing
                let checked: Vec<i32>;
                let units: &[i32] = if matches!(order, Some(Order::Move(_) | Order::Interact(_))) {
                    checked = self.checked_units(player, units);
                    if checked.iter().all(|&id| id == -1) && !self.players[player].computer {
                        return;
                    }
                    &checked
                } else if matches!(order, Some(Order::Guard(_))) {
                    // GUARD's check (FUN_140e550d0, FOLLOW's twin)
                    let Some(ids) = self.command_check(player, units) else {
                        return;
                    };
                    checked = ids;
                    &checked
                } else {
                    units
                };
                // FOLLOW (FUN_140e66a60) and GUARD (FUN_140e67620) of a target the world lacks do
                // nothing
                if let Some(Order::Follow(target) | Order::Guard(target)) = order
                    && self.object(target).is_none()
                {
                    return;
                }
                // +0x15: each unit with a unit AI takes the world time as its last human order
                // time (FUN_140e5e780), after the resets; FOLLOW sets it when a unit the world
                // holds belongs to a player who is not a computer (FUN_141659cf0)
                let human = match order {
                    Some(Order::Interact(o)) => o.human,
                    Some(Order::Move(o)) => o.human,
                    Some(Order::Follow(_) | Order::Guard(_)) => units.iter().any(|&id| {
                        self.object(id)
                            .is_some_and(|(p, _)| !self.players[p].computer)
                    }),
                    None => false,
                };
                self.reset_order_units(units);
                if human {
                    self.human_order(units);
                }
                let result = match order {
                    Some(Order::Interact(o)) => self.interact(player, units, &o),
                    Some(Order::Move(o)) => self.move_order(player, units, &o),
                    Some(Order::Follow(target)) => self.follow(player, units, target),
                    Some(Order::Guard(target)) => self.guard(player, units, target),
                    None => Err(GenieError::unsupported("an order moves its units")),
                };
                if let Err(e) = result {
                    self.stop_player(player, e);
                }
            }
        }
    }

    /// `FUN_140e5e780`: each unit of `ids` with a unit AI takes the world time as its last human
    /// order time (`+0x650`).
    pub(in crate::turn) fn human_order(&mut self, ids: &[i32]) {
        for &id in ids {
            let unit = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut())
                .find(|o| o.id == id)
                .filter(|o| o.ai.is_some());
            if let Some(u) = unit {
                u.last_human_order = self.world_time;
                self.spatial.set_gait(u.id, u.gait());
            }
        }
    }
}

impl Turns {
    /// `FUN_140e6cfc0`: each of an order's units (INTERACT, MOVE, RETREAT) the world holds,
    /// sleeping ones too (`FUN_1415417a0`), with a unit AI resets it for the order
    /// ([`UnitAi::reset_for_order`]), in list order.
    ///
    /// [`UnitAi::reset_for_order`]: crate::unit_ai::UnitAi::reset_for_order
    pub(in crate::turn) fn reset_order_units(&mut self, units: &[i32]) {
        for &id in units {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, i) = match place {
                crate::orders::Place::Live(p, i) | crate::orders::Place::Sleeping(p, i) => (p, i),
            };
            let player = &mut self.players[p];
            let unit = match place {
                crate::orders::Place::Live(..) => &mut player.objects[i],
                crate::orders::Place::Sleeping(..) => &mut player.sleeping[i],
            };
            if let Some(ai) = unit.ai.as_mut() {
                ai.reset_for_order(self.streams.get(Stream::Ai));
            }
        }
    }
}
