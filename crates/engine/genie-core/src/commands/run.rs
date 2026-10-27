//! A command's run in the command phase ([`World::run`]), a human player's order and the
//! order units' reset.

use crate::GenieError;
use crate::commands::{Command, Order, OrderUnits, QueueAt, ResearchAt};
use crate::math::rng::Stream;
use crate::world::World;

/// WORK's and RETREAT's runs (`FUN_140e6a9f0`, `FUN_140e69f00`).
const ORDER_UNMODELLED: GenieError = GenieError::unsupported("an order moves its units");

impl World {
    /// Run a command in the command phase (`TRIBE_Command::do_command`, `0x140e77b00`): the
    /// building commands ([`World::run_building_command`]), the unit commands
    /// ([`World::run_unit_command`]) and the players' ([`World::run_player_command`]). Where the
    /// slice does not model what the command does to the issuing player's units, that player's
    /// lane stops.
    pub(crate) fn run(&mut self, command: &Command) {
        let (player, result) = match command {
            Command::Build { .. }
            | Command::BuildWall(_)
            | Command::Queue { .. }
            | Command::Research { .. }
            | Command::GatherPoint { .. }
            | Command::CommandQueue { .. }
            | Command::Gate { .. }
            | Command::TownBell { .. }
            | Command::AllBackToWork { .. } => self.run_building_command(command),
            Command::UnitOrder { .. }
            | Command::Order { .. }
            | Command::Stop { .. }
            | Command::Patrol { .. }
            | Command::AttackMove { .. }
            | Command::SeekShelter { .. }
            | Command::UnitAiState { .. }
            | Command::AutoScout { .. }
            | Command::Unload { .. }
            | Command::GoBackToWork { .. }
            | Command::Delete { .. }
            | Command::TransformObject { .. } => self.run_unit_command(command),
            Command::Unmodelled { .. }
            | Command::Game(_)
            | Command::Trade { .. }
            | Command::Tribute { .. }
            | Command::Resign { .. }
            | Command::Flare { .. } => self.run_player_command(command),
        };
        if let Err(e) = result {
            self.stop_player(player, e);
        }
    }

    /// A building command's run ([`World::run`] routes them here): the player whose lane an
    /// error stops, and the run's result.
    fn run_building_command(&mut self, command: &Command) -> (usize, Result<(), GenieError>) {
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
            } => (
                player,
                self.build_foundation_command(
                    player,
                    (master, unit_ai),
                    position,
                    builders,
                    (queued, commits, frame),
                ),
            ),
            Command::BuildWall(ref w) => (w.player, self.build_wall_command(w)),
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
                (player, self.queue(player, &queue))
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
                (player, self.research_command(player, r))
            }
            Command::GatherPoint {
                player,
                ref ids,
                point,
                target,
                target_master,
                own,
            } => (
                player,
                self.gather_point(player, ids, point, (target, target_master), own),
            ),
            Command::CommandQueue {
                player,
                ref ids,
                target,
                point,
            } => (player, self.command_queue(player, ids, target, point)),
            Command::Gate { player, id } => (player, self.gate(player, id)),
            Command::TownBell {
                player,
                id,
                activate,
            } => (player, self.town_bell(player, id, activate)),
            Command::AllBackToWork { player, id } => (player, self.all_back_to_work(player, id)),
            _ => unreachable!("World::run routes only the building commands here"),
        }
    }

    /// A unit command's run ([`World::run`] routes them here): the player whose lane an error
    /// stops, and the run's result. A human player's unit order takes the world time as its
    /// units' last human order time first.
    fn run_unit_command(&mut self, command: &Command) -> (usize, Result<(), GenieError>) {
        match *command {
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
                (player, result)
            }
            Command::Order {
                player,
                ref units,
                order,
            } => (player, self.order_command(player, units, order)),
            Command::Stop { player, ref ids } => (player, self.stop_command(player, ids)),
            Command::Patrol {
                player,
                ref ids,
                ref waypoints,
                reset,
            } => (player, self.patrol(player, ids, waypoints, reset)),
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
                (player, self.attack_move(player, ids, waypoints, sweep))
            }
            Command::SeekShelter { player, ref ids } => (player, self.seek_shelter(player, ids)),
            Command::UnitAiState {
                player,
                ref ids,
                state,
            } => (player, self.unit_ai_state(player, ids, state)),
            Command::AutoScout { player, ref ids } => (player, self.auto_scout(player, ids)),
            Command::Unload {
                player,
                ref ids,
                point,
                flag,
                unit_type,
                sound,
            } => (
                player,
                self.unload(player, ids, point, flag, unit_type, sound),
            ),
            Command::GoBackToWork { player, ref ids } => {
                (player, self.go_back_to_work(player, ids))
            }
            Command::Delete { player, id } => (player, self.delete(player, id)),
            Command::TransformObject { player, id, master } => {
                (player, self.transform_object(player, id, master))
            }
            _ => unreachable!("World::run routes only the unit commands here"),
        }
    }

    /// A player's command ([`World::run`] routes them here): the player whose lane an error
    /// stops, and the run's result. What the slice cannot model of a GAME sub-command stops the
    /// player it is for (its issuer for none in the game); an unmodelled command stops its
    /// player; a trade cannot fail.
    fn run_player_command(&mut self, command: &Command) -> (usize, Result<(), GenieError>) {
        match *command {
            Command::Unmodelled { player, ref why } => (player, Err(why.clone())),
            Command::Game(ref g) => {
                let result = self.game(g);
                let p = if result.is_err() {
                    g.target(self.players.len()).unwrap_or(g.issuer)
                } else {
                    g.issuer
                };
                (p, result)
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
                (player, Ok(()))
            }
            Command::Tribute {
                player,
                to,
                ref gifts,
            } => (player, self.tribute(player, to, gifts)),
            Command::Resign { player, dropped } => (player, self.resign(player, dropped)),
            Command::Flare {
                player,
                target,
                point,
                players,
            } => (player, self.flare(target, point, players)),
            _ => unreachable!("World::run routes only the players' commands here"),
        }
    }

    /// BUILD (0x66): `do_command`'s case checks the builders first (another player's become
    /// −1; a human's BUILD left without one is refused). A farm on rice terrain takes its rice
    /// master; then the pending debit (`FUN_141630d10` with 0), the builders' test, the
    /// placement test and the payment: a refused BUILD places nothing. The foundation keeps the
    /// command's frame (`+0xc54`) before its builders get their tasks.
    fn build_foundation_command(
        &mut self,
        player: usize,
        (master, unit_ai): (i16, &Result<Option<u32>, GenieError>),
        position: [f32; 3],
        builders: &[i32],
        (queued, commits, frame): (bool, bool, u8),
    ) -> Result<(), GenieError> {
        let Some(builders) = self.command_check(player, builders) else {
            return Ok(());
        };
        let master = self.farm_on_rice_terrain(master, position);
        self.debit_listed(player, master)?;
        if !self.build_has_builder(player, master, &builders)? {
            return Ok(());
        }
        if !self.build_allowed(player, master, [position[0], position[1]])? {
            return Ok(());
        }
        let (id, position) = self.found(player, master, unit_ai.clone(), position)?;
        if let Some((_, u)) = self.object_mut(id) {
            u.build_frame = frame;
        }
        self.task_builders(player, id, master, position, &builders, (queued, commits))
    }

    /// `BUILD_WALL` (0x69): `do_command`'s case checks the builders as BUILD's, then the wall's
    /// line is placed.
    fn build_wall_command(&mut self, w: &crate::commands::wall::Wall) -> Result<(), GenieError> {
        let Some(builders) = self.command_check(w.player, &w.builders) else {
            return Ok(());
        };
        let w = crate::commands::wall::Wall {
            builders,
            ..w.clone()
        };
        self.build_wall(&w)
    }

    /// RESEARCH (0x65): `do_command`'s case checks the buildings first, then the research
    /// runs in those left.
    fn research_command(&mut self, player: usize, r: ResearchAt) -> Result<(), GenieError> {
        let Some(buildings) = self.research_check(player, &r) else {
            return Ok(());
        };
        self.research(player, &ResearchAt { buildings, ..r })
    }

    /// An order of units (INTERACT, WORK, MOVE, RETREAT, FOLLOW, GUARD): its units after the
    /// command's check ([`World::order_units`]). FOLLOW (`FUN_140e66a60`) and GUARD
    /// (`FUN_140e67620`) of a target the world lacks do nothing. The units reset for the order
    /// (WORK's not), a human's take the world time as their last human order time (`+0x15`,
    /// `FUN_140e5e780`; FOLLOW's when a unit the world holds belongs to a player who is not a
    /// computer, `FUN_141659cf0`), then the order runs.
    fn order_command(
        &mut self,
        player: usize,
        source: &OrderUnits,
        order: Option<Order>,
    ) -> Result<(), GenieError> {
        let Some(units) = self.order_units(player, source, order) else {
            return Ok(());
        };
        if order == Some(Order::Work) {
            return Err(ORDER_UNMODELLED);
        }
        if let Some(Order::Follow(target) | Order::Guard(target)) = order
            && self.object(target).is_none()
        {
            return Ok(());
        }
        let human = match order {
            Some(Order::Interact(o)) => o.human,
            Some(Order::Move(o)) => o.human,
            Some(Order::Follow(_) | Order::Guard(_)) => units.iter().any(|&id| {
                self.object(id)
                    .is_some_and(|(p, _)| !self.players[p].computer)
            }),
            Some(Order::Work) | None => false,
        };
        self.reset_order_units(&units);
        if human {
            self.human_order(&units);
        }
        match order {
            Some(Order::Interact(o)) => self.interact(player, &units, &o),
            Some(Order::Move(o)) => self.move_order(player, &units, &o),
            Some(Order::Follow(target)) => self.follow(player, &units, target),
            Some(Order::Guard(target)) => self.guard(player, &units, target),
            Some(Order::Work) | None => Err(ORDER_UNMODELLED),
        }
    }

    /// An order's units after its command's check. The checks of INTERACT (`FUN_140e55250`),
    /// WORK (`FUN_140e55eb0`), MOVE (`FUN_140e553f0`) and RETREAT (`FUN_140e55710`) read the
    /// units ([`World::command_units`]) and make each one the world does not hold, or of
    /// another player without the setting `+0x1074`, −1 ([`World::checked_units`]); read from
    /// the last commanded units, they leave the −1s there. A human player's command left
    /// without a unit, or a command of a player the world lacks, does nothing (None). Else the
    /// run reads its units again (`FUN_140e6b290` with 0): a list of the command's own becomes
    /// the last commanded units, −1s and all. FOLLOW and GUARD take the first 60 units of their
    /// packet (500 for a computer); GUARD's check is BUILD's builders' (`FUN_140e550d0`,
    /// [`World::command_check`]), FOLLOW's runs in [`World::follow`].
    ///
    /// [`World::command_units`]: crate::world::World::command_units
    /// [`World::checked_units`]: crate::world::World::checked_units
    /// [`World::command_check`]: crate::world::World::command_check
    fn order_units(
        &mut self,
        player: usize,
        source: &OrderUnits,
        order: Option<Order>,
    ) -> Option<Vec<i32>> {
        if let Some(Order::Follow(_) | Order::Guard(_)) = order {
            // the decoder lists FOLLOW's and GUARD's own units: their count is never 0xffff
            let ids = match source {
                OrderUnits::Listed(ids) => ids.as_slice(),
                OrderUnits::LastCommanded => &[],
            };
            let ids = &ids[..ids.len().min(self.most_units(player))];
            return match order {
                Some(Order::Guard(_)) => self.command_check(player, ids),
                _ => Some(ids.to_vec()),
            };
        }
        let computer = self.players.get(player)?.computer;
        let checked = self.checked_units(player, &self.command_units(player, source));
        if *source == OrderUnits::LastCommanded {
            self.set_last_commanded(player, checked.clone());
        }
        if !computer && checked.iter().all(|&id| id == -1) {
            return None;
        }
        if let OrderUnits::Listed(_) = source {
            self.set_last_commanded(player, checked.clone());
        }
        Some(checked)
    }

    /// `FUN_140e5e780`: each unit of `ids` with a unit AI takes the world time as its last human
    /// order time (`+0x650`).
    pub(crate) fn human_order(&mut self, ids: &[i32]) {
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

impl World {
    /// `FUN_140e6cfc0`: each of an order's units (INTERACT, MOVE, RETREAT) the world holds,
    /// sleeping ones too (`FUN_1415417a0`), with a unit AI resets it for the order
    /// ([`UnitAi::reset_for_order`]), in list order.
    ///
    /// [`UnitAi::reset_for_order`]: crate::unit_ai::UnitAi::reset_for_order
    pub(crate) fn reset_order_units(&mut self, units: &[i32]) {
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
