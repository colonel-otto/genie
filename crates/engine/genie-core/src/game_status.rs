//! The players' game status (`+0x89`: playing, won, lost) and what changes it: RESIGN
//! (`FUN_140d25af0`), the world's victory check after every update (`TRIBE_World` slot 0x68,
//! `FUN_1415c9480`, whose tail `FUN_14152e3d0` decides who won), and the status setter
//! (`WorldPlayer` slot 1, `FUN_141639860`) with a defeated player's stop of its objects
//! (`FUN_141613ed0`): its villagers, fishing boats and explorers stop and stand, its buildings
//! stop and give back their queues, and every unit AI's orders go.
//!
//! 158041: the game over test `FUN_14152f810`, a player's own victory `FUN_14165d720`, the queue's
//! cancel `FUN_1411cff00`, the make action's destructor `FUN_140ace3f0`.

use crate::rng::Stream;
use crate::turn::Turns;
use crate::unit_ai::{Effect, Unsupported};

/// The player still plays.
pub const PLAYING: u8 = 0;
/// The player won.
pub const WON: u8 = 1;
/// The player lost (or resigned).
pub const LOST: u8 = 2;

/// The game mode whose status changes shrink the map (battle royale, 12).
const BATTLE_ROYALE: i32 = 12;
/// A defeated player's objects stop once the game is this old, in milliseconds (`FUN_141639860`;
/// in a recording's playback the session check that would stop them earlier passes).
const STOP_AFTER_MS: u32 = 60_000;
/// The classes a defeat stops besides explorers: villagers (4) and fishing boats (21).
const VILLAGER: i16 = 4;
const FISHING_BOAT: i16 = 21;
/// The defeat's event weighs the players below this id (`FUN_141639860`'s `CMP R14D, 0x19`).
const DEFEAT_IDS: usize = 25;
/// The first of the attributes a defeat's event reads (300 + a player's id).
const KILLS_OF: usize = 300;
/// The class whose unit AI a stop leaves alone (`FUN_14108c050`).
const SCOUT_CLASS: i16 = 58;
/// The trade carts' class, which keep their actions while they carry goods (`FUN_14106a940`).
const TRADE_CART: i16 = 19;
/// The unit AI action value a stop leaves alone (`FUN_14108c050`).
const KEPT_ACTION: i32 = 0x2c2;
/// The trade action's type (`0x6f`), whose load keeps a stopped unit's AI going.
const TRADE_ACTION: i16 = 0x6f;
/// The action types whose stop keeps a value of the unit's (`+0x88a`, `FUN_14106fa60`).
const KEEPING_TYPES: [i16; 2] = [0x7c, 0x7d];
/// The buildings a stop leaves alone (`FUN_1411e6290`).
const UNSTOPPED_BUILDINGS: [i16; 4] = [50, 199, 276, 1187];
/// The masters of type 70 and up have their costs given back (`FUN_141630530`: type above 0x45).
const REFUNDED_TYPE: u8 = 70;

/// What the world's checks count of the players (Gaia aside): those still playing and whether
/// they form one side (0 none, 1 one, −1 more), and those who won and lost.
#[derive(Clone, Copy, Debug, Default)]
struct Count {
    playing: u32,
    first: usize,
    side: i32,
    won: u32,
    lost: u32,
}

impl Count {
    /// Player `i` of `status`, with the allied victory flag and the relations both ways to the
    /// first player still playing (`relation` 0: allied; `FUN_141667f70`).
    const fn add(&mut self, i: usize, status: u8, allied_victory: bool, relations: (u8, u8)) {
        match status {
            PLAYING => {
                self.playing += 1;
                if self.first != 0 && self.side != 0 {
                    // another side unless the two count each other's victory as their own
                    if !allied_victory || relations.0 != 0 || relations.1 != 0 {
                        self.side = -1;
                    }
                } else {
                    self.side = 1;
                    self.first = i;
                }
            }
            WON => self.won += 1,
            LOST => self.lost += 1,
            _ => {}
        }
    }
}

impl Turns {
    /// The relations of players `a` and `b` each way (`player+0x260`).
    fn relations(&self, a: usize, b: usize) -> (u8, u8) {
        let r = |x: usize, y: usize| {
            self.players
                .get(x)
                .and_then(|p| p.relations.get(y))
                .copied()
                .unwrap_or(0)
        };
        (r(a, b), r(b, a))
    }

    /// `FUN_14152f810`: the game is over: the world's game state is 1 or a player won; else,
    /// under the conquest victory, the players still playing form one side with someone lost,
    /// or nobody plays. A player's `+0x8a`, which would count it lost, is not held (0).
    pub(crate) fn game_ended(&self) -> bool {
        if self.game_state == 1 {
            return true;
        }
        let mut count = Count::default();
        for (i, p) in self.players.iter().enumerate().skip(1) {
            let rel = self.relations(count.first, i);
            count.add(i, p.game_status, p.allied_victory, rel);
        }
        if 0 < count.won {
            return true;
        }
        self.victory_conquest && ((0 < count.lost && 0 < count.side) || count.playing == 0)
    }

    /// RESIGN (`FUN_140d25af0`): the comm player resigns (its flags are the comm handler's);
    /// once every comm player of its world player has (one each in the slice's games), the
    /// world player's slot 3 (`FUN_1416610f0`): `+0x8c` takes the drop, `+0x8b` is set and the
    /// status becomes lost ([`Turns::set_status`]); its message is the interface's.
    pub(crate) fn resign(&mut self, player: usize, dropped: bool) -> Result<(), Unsupported> {
        let p = self
            .players
            .get_mut(player)
            .ok_or(Unsupported("RESIGN of a player the slice does not have"))?;
        p.dropped = dropped;
        p.resigned = true;
        self.set_status(player, LOST)
    }

    /// The status setter (slot 1, `FUN_141639860`) of player `p`: `FUN_1416768e0` takes the new
    /// status unless the player resigned (a defeat it always takes; one without a resignation
    /// would tell the comm handler, which the slice does not hold). A status that turned won or
    /// lost while the game goes on reveals the map to the player (`FUN_1412847b0`, with no lobby
    /// settings object or its setting `FUN_141300cd0`, as the playback holds it,
    /// [`Spatial::reveal_to`]); the battle royale's is not modelled. A lost player's objects stop
    /// ([`Turns::stop_everything`]) once the game is a minute old; its fealty (`+0x12f8`) would
    /// give its resources and units away while the game goes on, which is not modelled. A
    /// newly lost player's history takes its defeat ([`Turns::defeat_event`]).
    pub(crate) fn set_status(&mut self, p: usize, status: u8) -> Result<(), Unsupported> {
        let old = self.players[p].game_status;
        if status != old && (!self.players[p].resigned || status == LOST) {
            if status == LOST && !self.players[p].resigned {
                return Err(Unsupported(
                    "a defeat without a resignation (its comm players, FUN_1416768e0)",
                ));
            }
            self.players[p].game_status = status;
        }
        let now = self.players[p].game_status;
        if now != old && matches!(now, WON | LOST) && !self.game_ended() {
            // the map revealed to the player (FUN_1412847b0 with 0x4000000040000000): every
            // tile of the unified map and the player's masks take the ended bits; the view's
            // refresh (FUN_140c80bb0) is not held
            self.spatial.reveal_to(p);
        }
        if self.game_mode == BATTLE_ROYALE {
            return Err(Unsupported(
                "a status change in a battle royale (game mode 12)",
            ));
        }
        if now == LOST {
            if STOP_AFTER_MS < self.world_time {
                self.stop_everything(p)?;
            }
            if old != LOST {
                self.defeat_event(p);
            }
            if old != LOST && 0 < self.players[p].fealty && self.game_state == 0 {
                return Err(Unsupported(
                    "a defeated player's fealty (its resources and units given away)",
                ));
            }
        }
        Ok(())
    }

    /// The defeat's history event (`FUN_141639860` from `0x141639cd1`) of player `p`, unless it
    /// resigned (`+0x8b`) or is Gaia: the other players but Gaia (ids up to 24) are weighed by
    /// their attribute 300 + their own id (the game reads each one's own, not the defeated
    /// player's); the first strictly above the best so far (`COMISS` with `JBE`, from 0) is the
    /// event's `data1`, its value through `CVTTSS2SI` its `data2`; none gives player 1 and 0.
    pub(crate) fn defeat_event(&mut self, p: usize) {
        if self.players[p].resigned || p == 0 {
            return;
        }
        let mut killer = 1u8;
        let mut most = 0i32;
        for (i, other) in self.players.iter().enumerate() {
            if i == 0 || i == p || DEFEAT_IDS <= i {
                continue;
            }
            let v = other.attributes.get(KILLS_OF + i).copied().unwrap_or(0.0);
            if v > most as f32 {
                most = crate::checkpoint::cvttss2si(v);
                killer = i as u8;
            }
        }
        let now = self.world_time;
        self.players[p].history.add_event(
            crate::history::DEFEAT,
            f32::from(killer),
            most as f32,
            0,
            now,
        );
    }

    /// `FUN_14165d720`: player `i`'s status for the victory check. A player still playing whose
    /// victory conditions are met (`FUN_1414f35d0`) wins (slot 1 with won; the world's `+0x8c`,
    /// 2, is not held); one counting its allies' victory as its own is not modelled.
    fn own_victory(&mut self, i: usize) -> Result<u8, Unsupported> {
        let p = &self.players[i];
        if p.game_status == LOST || p.victory != 2 {
            return Ok(p.game_status);
        }
        if p.allied_victory {
            return Err(Unsupported(
                "a player's victory with its allies' (allied victory)",
            ));
        }
        self.set_status(i, WON)?;
        Ok(self.players[i].game_status)
    }

    /// The world's victory check after every update (`TRIBE_World` slot 0x68, `FUN_1415c9480`):
    /// its relic and wonder countdowns (`world+0x840`..`+0x8d9`) are not modelled; its tail
    /// (`FUN_14152e3d0` with 1, 0) counts the players by their own victory
    /// ([`Turns::own_victory`]). Someone won: the game is over (game state 1) and those still
    /// playing lose (`FUN_141666590`). Else under the conquest victory, one side left with
    /// someone lost, or nobody playing: the game is over and those still playing win
    /// (`FUN_1416778c0`; the world's `+0x8c`, 1, is not held).
    pub(crate) fn victory_check(&mut self) {
        let mut count = Count::default();
        for i in 1..self.players.len() {
            let status = match self.own_victory(i) {
                Ok(s) => s,
                Err(e) => {
                    self.stop_player(i, e);
                    return;
                }
            };
            let rel = self.relations(count.first, i);
            let allied_victory = self.players[i].allied_victory;
            count.add(i, status, allied_victory, rel);
        }
        let outcome = if 0 < count.won {
            LOST
        } else if self.victory_conquest
            && ((0 < count.lost && 0 < count.side) || count.playing == 0)
        {
            WON
        } else {
            return;
        };
        self.game_state = 1;
        for i in 1..self.players.len() {
            if self.players[i].game_status == PLAYING
                && let Err(e) = self.set_status(i, outcome)
            {
                self.stop_player(i, e);
            }
        }
    }

    /// `FUN_141613ed0`: a defeated player's objects in its live list's order. A villager, a
    /// fishing boat and an explorer (unit AI action `0x2bd` or 605) empty their ignore lists
    /// (`FUN_141197da0`), stop (slot 0x148, [`Turns::object_stop`]) and stand (slot 0x560:
    /// the stop of their movement, slot 0x5d0, and their walked level's reset,
    /// `FUN_140f98e80`); a building stops but for a few masters (`FUN_1411e6290`) and gives
    /// back its queue ([`Turns::cancel_queue`]); every unit AI's orders go (`FUN_140c5e010`). The
    /// slice holds no unit groups (`+0x6d0`).
    fn stop_everything(&mut self, p: usize) -> Result<(), Unsupported> {
        use crate::explore::EXPLORE_ACTION;
        use crate::unit_ai::AUTO_SCOUT_ACTION;
        for idx in 0..self.players[p].objects.len() {
            let unit = &self.players[p].objects[idx];
            let master = self.master_of(p, unit.life.master_id)?;
            let (class, id) = (master.class, master.id);
            let explorer = unit
                .ai
                .as_ref()
                .is_some_and(|ai| matches!(ai.current_action, AUTO_SCOUT_ACTION | EXPLORE_ACTION));
            if matches!(class, VILLAGER | FISHING_BOAT) || explorer {
                if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                    m.ignore_units.clear();
                }
                self.object_stop(p, idx)?;
                if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                    m.stand()?;
                }
            }
            if class == crate::garrison::BUILDING_CLASS {
                if !UNSTOPPED_BUILDINGS.contains(&id) {
                    self.object_stop(p, idx)?;
                }
                self.cancel_queue(p, idx)?;
            }
            if let Some(ai) = self.players[p].objects[idx].ai.as_mut() {
                ai.orders.clear();
            }
        }
        Ok(())
    }

    /// The object stop (slot 0x148, `FUN_14108c050`; a building's `FUN_1411e6290`) of the object
    /// at `idx` of player `p`: the action object's stop (`FUN_14106fa60`): its top action's
    /// slot 0x38 ([`Turns::halt_top_action`]); the action list goes ([`Turns::clear_actions`])
    /// unless that answered true for a unit that is no trade cart with goods; then a unit AI of
    /// a class other than 58 whose action value is not `0x2c2` and whose top action is no trade
    /// (0x6f) with a load stops (slot 0xb0 with 0: the order ends, and stopAction, slot 0x3c0)
    /// and forgets its target and its last target (`FUN_140c4dcb0`).
    pub(crate) fn object_stop(&mut self, p: usize, idx: usize) -> Result<(), Unsupported> {
        let unit = &self.players[p].objects[idx];
        let master = self.master_of(p, unit.life.master_id)?;
        let class = master.class;
        if unit
            .top_action_kind()
            .is_some_and(|k| KEEPING_TYPES.contains(&k))
        {
            return Err(Unsupported("a stop of an action of type 0x7c or 0x7d"));
        }
        let keeps = self.halt_top_action(p, idx)?;
        if keeps && class == TRADE_CART {
            return Err(Unsupported(
                "a trade cart's goods after its stop (+0x7a0, FUN_141188160)",
            ));
        }
        // FUN_14106fa60: the list stays when the top action's slot 0x38 answered true and the
        // unit is no trade cart with goods (FUN_14106a940)
        if !keeps {
            self.clear_actions(p, idx)?;
        }
        let unit = &mut self.players[p].objects[idx];
        let place = ([unit.position[0], unit.position[1]], class);
        // slot 0x438 is the top action's type: a trade (0x6f) with a load held (COMISS 0, held;
        // JNC at 0x14108c11e: a NaN load counts) keeps the unit AI going
        let trading = unit.top_action_kind() == Some(TRADE_ACTION)
            && crate::fmath::below_or_unordered(0.0, unit.life.amount)
            && unit.life.attribute_type != -1;
        let Some(ai) = unit.ai.as_mut() else {
            return Ok(());
        };
        if class == SCOUT_CLASS || ai.current_action == KEPT_ACTION || trading {
            return Ok(());
        }
        let mut fx = Vec::new();
        ai.stop_with(false, Some(place), &mut fx)?;
        for e in fx {
            match e {
                Effect::Stop => self.command_stop(p, idx)?,
                _ => return Err(Unsupported("a stop's effect not modelled")),
            }
        }
        if let Some(ai) = self.players[p].objects[idx].ai.as_mut() {
            ai.forget_targets();
        }
        Ok(())
    }

    /// The top action's slot 0x38 before the stop clears the list (`FUN_140a97770`): false
    /// without one, or for a `MoveTo`, an Explore or an Enter (the base's, `FUN_140a7bbe0`). A
    /// human player's Hunt (`FUN_140ac79b0`) goes to state 2 (`0x140ac60e0` case 2): its
    /// sub-actions go, the unit stands with the hunt's sprite (slot 0x68 with
    /// `FUN_140ac5210`'s), its target goes (slot 0x68 with none, [`Turns::release`]) and its
    /// second target (`FUN_140a7a190` with none), its point and timer clear; then the pasture it
    /// tends lets it go (`FUN_141189ca0` with −1), and the answer is true. The Fly (Bird), the
    /// missile's, the Make and the wait for a foundation keep the base's too; the Gather, the
    /// Build, the Attack and the Artifact halt ([`Turns::halt_other`]).
    fn halt_top_action(&mut self, p: usize, idx: usize) -> Result<bool, Unsupported> {
        let computer = self.players[p].computer;
        let by = self.releaser(p, idx);
        let master_id = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master_id)?.standing_sprite;
        let unit = &mut self.players[p].objects[idx];
        let Some(kind) = unit.top_action_kind() else {
            return Ok(false);
        };
        match kind {
            crate::move_to::MOVE_TO
            | crate::explore::EXPLORE
            | crate::garrison::ENTER
            | crate::fly::FLY
            | crate::missile::MISSILE
            | crate::make::MAKE
            | crate::wait_solidify::WAIT_SOLIDIFY => {
                return Ok(false);
            }
            crate::hunt::HUNT => {}
            _ => return self.halt_other(p, idx, kind),
        }
        if computer {
            return Err(Unsupported(
                "a computer player's hunt stopped (set_state 1)",
            ));
        }
        let hunt = unit.hunt.as_mut().expect("its top action");
        hunt.tear_down(unit.mover.as_mut())?;
        hunt.state = crate::hunt::IDLE;
        let sprite = crate::hunt::stand_sprite(
            &hunt.task,
            standing,
            unit.life.attribute_type,
            unit.life.amount,
            &self.graphics,
        );
        // the target, then the second target
        let released = [hunt.target, hunt.carry_to];
        hunt.target = -1;
        hunt.carry_to = -1;
        hunt.point = [-1.0; 3];
        hunt.timer = 0.0;
        if let Some(g) = sprite {
            crate::attack::set_sprite(&mut unit.sprite, g, self.streams.get(Stream::Sprite));
        }
        if unit.my_pasture != -1 {
            return Err(Unsupported("a stopped hunter that tends a pasture"));
        }
        for t in released {
            if t != -1 {
                self.release(t, by)?;
            }
        }
        Ok(true)
    }

    /// Slot 0x38 of the top actions with their own besides the Hunt's, each true: the Gather's
    /// (`FUN_140a8e1b0`, [`crate::gather::GatherAction::halt`]: its target and second target go,
    /// [`Turns::release`]), the Build's (`FUN_140ab0350`, [`crate::build::BuildAction::halt`]:
    /// a counted builder leaves its foundation's builders, [`Turns::leave_builders`], before the
    /// rest; the foundation goes), the Attack's (`FUN_140a82ac0`,
    /// [`crate::attack::Attack::halt`]) and the Artifact's (`FUN_140aad1a0`,
    /// [`crate::artifact::Artifact::stop`]). The Gather, the Build and the Artifact show the
    /// master's standing sprite (the Gather the task's carrying one while it carries at least 1 of
    /// the task's resource, `FUN_140a8b6b0`). [`crate::turn::Unit::top_action`] holds no other
    /// kind.
    fn halt_other(&mut self, p: usize, idx: usize, kind: i16) -> Result<bool, Unsupported> {
        let by = self.releaser(p, idx);
        let master_id = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master_id)?.standing_sprite;
        match kind {
            crate::gather::GATHER => {
                let unit = &mut self.players[p].objects[idx];
                let gather = unit.gather.as_mut().expect("its top action");
                let stand = crate::hunt::stand_sprite(
                    &gather.task,
                    standing,
                    unit.life.attribute_type,
                    unit.life.amount,
                    &self.graphics,
                );
                let released = gather.halt(
                    unit.mover.as_mut(),
                    stand,
                    &mut unit.sprite,
                    self.streams.get(Stream::Sprite),
                )?;
                for t in released {
                    if t != -1 {
                        self.release(t, by)?;
                    }
                }
            }
            crate::build::BUILD => {
                let unit = &mut self.players[p].objects[idx];
                let build = unit.build.as_mut().expect("its top action");
                let foundation = build.target;
                let (counted, target) = build.halt(unit.mover.as_mut())?;
                if counted {
                    self.leave_builders(foundation)?;
                }
                self.standing_sprite(p, idx)?;
                if target != -1 {
                    self.release(target, by)?;
                }
            }
            crate::attack::ATTACK => {
                let unit = &mut self.players[p].objects[idx];
                let attack = unit.attack.as_mut().expect("its top action");
                attack.halt(
                    unit.mover.as_mut(),
                    &mut unit.anim_speed,
                    &mut unit.sprite,
                    self.streams.get(Stream::Sprite),
                )?;
            }
            crate::artifact::ARTIFACT_TYPE => {
                let unit = &mut self.players[p].objects[idx];
                let artifact = unit.artifact.as_mut().expect("its top action");
                if artifact.stop() {
                    unit.mover
                        .as_mut()
                        .ok_or(Unsupported("a walking animal without movement state"))?
                        .stop()?;
                }
                self.standing_sprite(p, idx)?;
            }
            _ => {
                return Err(Unsupported(
                    "a stop of an action whose slot 0x38 is not modelled",
                ));
            }
        }
        Ok(true)
    }

    /// `FUN_1411cff00`, the cancel of the queue of the building at `idx` of player `p`: each
    /// unit queued gives back what its entry paid of its master's paid costs (`FUN_141630530`,
    /// masters of type 70 and up) and its population leaves attribute 80; the player's queue
    /// counts (`+0x18`) are not held, nor is the building's `+0xc00`. A research queued is not
    /// modelled. Then the make action goes (`FUN_140a98620` on `+0xbf8`), its destructor
    /// (`FUN_140ace3f0`): while the game is not over its unit's population leaves attribute 81
    /// and, made without the queue's payment (`+0x60` clear), the master's paid costs come back
    /// (`FUN_141630770`); the building's unit AI is told the make action completed (506 with
    /// 619).
    fn cancel_queue(&mut self, p: usize, idx: usize) -> Result<(), Unsupported> {
        let Some(production) = self.players[p].objects[idx].production.clone() else {
            return Ok(());
        };
        if production.queue.iter().any(|q| q.tech != -1) {
            return Err(Unsupported("a cancelled queue's research (FUN_1413d15e0)"));
        }
        let front = production.queue.front().map(|q| q.master);
        for q in &production.queue {
            self.refund_queued(p, q)?;
        }
        self.drop_make_action(p, idx, &production, front)?;
        if let Some(production) = self.players[p].objects[idx].production.as_mut() {
            production.queue.clear();
            production.action = false;
            production.progress = 0.0;
        }
        Ok(())
    }

    /// A queued unit's refund (`FUN_141630530`, masters of type 70 and up): its entry's amount of
    /// each of its master's paid costs comes back; its population leaves attribute 80.
    fn refund_queued(
        &mut self,
        p: usize,
        q: &crate::production::Queued,
    ) -> Result<(), Unsupported> {
        use crate::production::{QUEUED, population};
        let master = self.master_of(p, q.master)?.clone();
        if REFUNDED_TYPE <= master.kind {
            for &(resource, _, paid) in &master.cost {
                if resource < 0 || !paid {
                    continue;
                }
                let back = q
                    .costs
                    .iter()
                    .rev()
                    .find(|c| c.0 != -1 && c.0 == i32::from(resource))
                    .map_or(0.0, |c| c.1);
                self.players[p].add_attribute(resource, back)?;
            }
        }
        self.players[p].add_attribute(QUEUED, -population(&master.storage))
    }

    /// `UNIT_ORDER`'s action 4 (`FUN_14161e370` case 4 -> `FUN_1411d66b0`) at the building at
    /// `idx`: the `index`-th unit of its queue (by unit, across its entries) is cancelled: its
    /// refund ([`Turns::refund_queued`]) and the queue lets it go (the player's queue counts,
    /// `+0x18`, and the building's `+0xbf4` and `+0xc00` are not held); the first one's make
    /// action goes too ([`Turns::drop_make_action`]; the interface's `+0xc02` is not the sim's).
    /// An index past the queue does nothing. A research's cancel and the cancel of up to five of
    /// a kind (`all`, the command's `+0x1d`: `FUN_1411d61e0`) are not modelled, nor is game mode
    /// 16's tail.
    pub(crate) fn cancel_queued(
        &mut self,
        p: usize,
        idx: usize,
        index: usize,
        all: bool,
    ) -> Result<(), Unsupported> {
        let Some(production) = self.players[p].objects[idx].production.clone() else {
            return Ok(());
        };
        let Some(&q) = production.queue.get(index) else {
            return Ok(());
        };
        if q.tech != -1 {
            return self.cancel_research(p, idx, index, all, &production);
        }
        if all {
            return Err(Unsupported(
                "a cancel of up to five of a kind (FUN_1411d61e0)",
            ));
        }
        if self.game_mode == 16 {
            return Err(Unsupported("a cancel's tail in game mode 16"));
        }
        self.refund_queued(p, &q)?;
        if let Some(production) = self.players[p].objects[idx].production.as_mut() {
            let _ = production.queue.remove(index);
        }
        if index == 0 {
            self.drop_make_action(p, idx, &production, Some(q.master))?;
            if let Some(production) = self.players[p].objects[idx].production.as_mut() {
                production.action = false;
                production.progress = 0.0;
            }
        }
        Ok(())
    }

    /// A research's cancel (`FUN_1411d66b0`'s branch for a technology, then `FUN_1411d61e0` with
    /// it and one): the first item's research action, when it runs, loses its progress
    /// (`+0x70`) and the technology its work done; the queue entry's payment comes back
    /// (`FUN_1413d15e0`: each cost entry's amount to its attribute) and the technology is in state
    /// 1 again with no work done; its researches queued (`+0x58`) lose one and the queue lets it
    /// go. For the first item the research action goes (`FUN_140a98620`): its destructor
    /// (`FUN_140ad0be0`, while the game is not over) finds nothing left to refund
    /// (`FUN_1413ccc00` on a technology in state 1) and tells the building's unit AI the action
    /// failed (`0x1f9` with `0x26c`). Not held: the building's list of its researches (`+0xd00`,
    /// `FUN_1411c5250`), the interface's `+0xc02`; not modelled: costs the building paid (flag 2,
    /// `FUN_1411c7a00`), the cancel of up to five (`all`), a repeatable technology's tail of the
    /// destructor.
    fn cancel_research(
        &mut self,
        p: usize,
        idx: usize,
        index: usize,
        all: bool,
        production: &crate::production::Production,
    ) -> Result<(), Unsupported> {
        if all {
            return Err(Unsupported(
                "a cancel of up to five researches (FUN_1411d61e0)",
            ));
        }
        let q = production.queue[index];
        let t = usize::try_from(q.tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len())
            .ok_or(Unsupported("a cancelled research beyond the tech tree"))?;
        let def = self
            .techs
            .get(t)
            .ok_or(Unsupported("a technology the data file has not got"))?;
        if def.costs.iter().any(|c| c.paid == 2) {
            return Err(Unsupported(
                "a cancelled research whose costs the building paid (FUN_1411c7a00)",
            ));
        }
        let started = index == 0 && production.action;
        // the player's +0x648, which the destructor's tail also reads, is 0 for every player probed
        if started && self.players[p].techs[t].repeatable(def, false) {
            return Err(Unsupported(
                "a cancelled repeatable research (its action's destructor's tail)",
            ));
        }
        for &(attribute, amount) in &q.costs {
            if let Some(attribute) = i16::try_from(attribute).ok().filter(|&a| 0 <= a) {
                self.players[p].add_attribute(attribute, amount)?;
            }
        }
        let tech = &mut self.players[p].techs[t];
        tech.research_done = 0.0;
        tech.state = 1;
        tech.pending -= 1;
        let building = &mut self.players[p].objects[idx];
        if let Some(production) = building.production.as_mut() {
            let _ = production.queue.remove(index);
            if started {
                production.action = false;
                production.progress = 0.0;
            }
        }
        if started && self.game_state == 0 {
            let (id, state) = (building.id, building.life.state);
            if let Some(ai) = building.ai.as_mut() {
                let n = crate::unit_ai::Notify::own(
                    id,
                    crate::unit_ai::ACTION_FAILED,
                    [crate::turn::RESEARCH_ACTION, 0, 0],
                );
                ai.post(n, state)?;
            }
        }
        Ok(())
    }

    /// The building's make action goes (`FUN_140a98620` on `+0xbf8`), its destructor
    /// (`FUN_140ace3f0`): while the game is not over its unit's (`front`) population leaves
    /// attribute 81 and, made without the queue's payment (`+0x60` clear), the master's paid costs
    /// come back (`FUN_141630770`); the building's unit AI is told the make action completed (506
    /// with 619).
    fn drop_make_action(
        &mut self,
        p: usize,
        idx: usize,
        production: &crate::production::Production,
        front: Option<i16>,
    ) -> Result<(), Unsupported> {
        use crate::production::{IN_PRODUCTION, population};
        if production.action {
            let master = self
                .master_of(
                    p,
                    front.ok_or(Unsupported("a make action without a unit queued"))?,
                )?
                .clone();
            if self.game_state == 0 {
                self.players[p].add_attribute(IN_PRODUCTION, -population(&master.storage))?;
                if !production.make_paid && REFUNDED_TYPE <= master.kind {
                    for &(resource, amount, paid) in &master.cost {
                        if 0 <= resource && paid {
                            self.players[p].add_attribute(resource, f32::from(amount))?;
                        }
                    }
                }
            }
            let building = &mut self.players[p].objects[idx];
            let (id, state) = (building.id, building.life.state);
            if let Some(ai) = building.ai.as_mut() {
                let n = crate::unit_ai::Notify::own(
                    id,
                    crate::unit_ai::ACTION_COMPLETED,
                    [crate::spawn::MAKE_ACTION, 0, 0],
                );
                ai.post(n, state)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::{Master, Masters};
    use crate::production::{Production, Queued};
    use crate::turn::{Player, Unit};
    use crate::unit_ai::UnitAi;

    /// Gaia and two players; player 1's town centre (class 3) makes a villager its queue paid
    /// 46 food for, a minute and more into the game.
    fn game() -> Turns {
        let mut attributes = vec![0.0; 250];
        (attributes[80], attributes[81]) = (1.0, 1.0);
        let tc = Unit {
            id: 9,
            ai: Some(UnitAi {
                current_order: -1,
                current_action: 0x26b,
                current_target: -1,
                ..UnitAi::default()
            }),
            production: Some(Production {
                queue: [Queued {
                    master: 83,
                    tech: -1,
                    train_time: 25.0,
                    costs: [(0, 46.0), (-1, 0.0), (4, 1.0)],
                }]
                .into(),
                action: true,
                make_paid: true,
                progress: 3.0,
                rate: 1.0,
            }),
            ..Unit::default()
        };
        let player = |objects| Player {
            objects,
            attributes: attributes.clone(),
            relations: vec![0, 0, 3],
            ..Player::default()
        };
        let mut t = Turns::new(
            61_000,
            1,
            0,
            vec![0; 3],
            vec![Player::default(), player(vec![tc]), player(vec![])],
        );
        let masters = Masters::new([
            Master {
                id: 109,
                class: 3,
                kind: 80,
                ..Master::default()
            },
            Master {
                id: 83,
                class: 4,
                kind: 70,
                cost: vec![(0, 46, true), (4, 1, false)],
                storage: vec![(4, -1.0, 2), (11, 1.0, 2)],
                ..Master::default()
            },
        ]);
        t.players[1].objects[0].life.master_id = 109;
        t.masters = vec![masters.clone(), masters.clone(), masters];
        t.victory_conquest = true;
        t
    }

    /// The action object's stop (`FUN_14106fa60`) keeps the action list when the top action's
    /// slot 0x38 answers true and the unit is no trade cart with goods (`FUN_14106a940`): a
    /// human hunter's Hunt halts (state 2) and stays. A unit AI then stops (slot 0xb0) as the
    /// top action is no trade (0x6f), and its stopAction empties the list.
    /// A defeat (`FUN_141639860`) adds its event to the history: the player with the most of
    /// its own attribute 300 + id, Gaia and the defeated player left out; a resigned player and
    /// a history without entries get none.
    #[test]
    fn a_defeat_names_the_player_with_the_most_of_its_own_attribute() {
        let mut t = game();
        let other = t.players[1].clone();
        t.players.push(other);
        t.players[2].attributes.resize(310, 0.0);
        t.players[3].attributes.resize(310, 0.0);
        t.players[1].attributes.resize(310, 0.0);
        t.players[0].attributes = vec![9.0; 310];
        t.players[2].attributes[302] = 3.0;
        t.players[3].attributes[303] = 7.9;
        t.players[1].attributes[301] = 50.0;
        t.players[1].history.entries = vec![[1, 0]; 4];
        t.world_time = 5_000;
        t.defeat_event(1);
        assert_eq!(
            t.players[1].history.events,
            [crate::history::Event {
                kind: crate::history::DEFEAT,
                time_slice: 4,
                world_time: 5_000,
                data1: 3.0,
                data2: 7.0,
                data3: 0.0,
            }]
        );
        t.players[2].resigned = true;
        t.players[2].history.entries = vec![[1, 0]];
        t.defeat_event(2);
        assert_eq!(t.players[2].history.events, []);
    }

    #[test]
    fn a_stopped_hunter_keeps_its_halted_hunt() {
        use crate::hunt::{HuntAction, HuntTask, Lead};
        let hunter = Unit {
            id: 20,
            ai: Some(UnitAi {
                current_order: 0x2c9,
                current_action: 0x265,
                current_target: -1,
                ..UnitAi::default()
            }),
            hunt: Some(HuntAction {
                target: -1,
                state: 4,
                point: [-1.0; 3],
                target_class: -1,
                task: HuntTask {
                    id: 0,
                    range: 0.0,
                    resource: -1,
                    sprites: [-1; 4],
                    work_value: 0.0,
                    productivity: -1,
                    kind4: -1,
                },
                attack: None,
                move_to: None,
                walk_range: 0.0,
                drop_site: -1,
                carry_to: -1,
                timer: 0.0,
                move_wait: crate::move_to::MoveWait::default(),
                lead: Lead::default(),
            }),
            ..Unit::default()
        };
        let mut t = game();
        let mut hunter = hunter;
        // game()'s villager master
        hunter.life.master_id = 109;
        hunter.my_pasture = -1;
        t.players[1].objects.push(hunter);
        let idx = t.players[1].objects.len() - 1;
        let mut without_ai = t.clone();
        // the unit AI's stopAction (slot 0x3c0) then empties the list
        t.object_stop(1, idx).unwrap();
        let u = &t.players[1].objects[idx];
        assert_eq!(u.hunt, None);
        assert_eq!(u.ai.as_ref().unwrap().current_action, -1);
        // without a unit AI the halted Hunt stays
        without_ai.players[1].objects[idx].ai = None;
        without_ai.object_stop(1, idx).unwrap();
        let u = &without_ai.players[1].objects[idx];
        assert_eq!(u.hunt.map(|h| h.state), Some(crate::hunt::IDLE));
    }

    /// records/425906646's player 2 cancels the second villager of its town centre at update
    /// 3699 (`UNIT_ORDER` action 4, index 1): that one's 46 food come back and its population
    /// leaves attribute 80; the first keeps its make action. A cancel of the first drops the make
    /// action too; an index past the queue does nothing.
    #[test]
    fn a_cancel_gives_back_one_queued_unit() {
        let mut t = game();
        let second = Queued {
            costs: [(0, 45.0), (-1, 0.0), (4, 1.0)],
            ..t.players[1].objects[0].production.as_ref().unwrap().queue[0]
        };
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue.push_back(second);
        t.players[1].attributes[80] = 2.0;
        t.cancel_order(&[9], 1, false).unwrap();
        let p = &t.players[1];
        assert_eq!((p.resources[0], p.attributes[80]), (45.0, 1.0));
        let production = p.objects[0].production.as_ref().unwrap();
        assert_eq!((production.queue.len(), production.action), (1, true));
        t.cancel_order(&[9], 5, false).unwrap();
        assert_eq!(t.players[1].resources[0], 45.0);
        t.cancel_order(&[9], 0, false).unwrap();
        let p = &t.players[1];
        assert_eq!(
            (p.resources[0], p.attributes[80], p.attributes[81]),
            (91.0, 0.0, 0.0)
        );
        let production = p.objects[0].production.as_ref().unwrap();
        assert!(production.queue.is_empty() && !production.action);
        assert!(t.cancel_order(&[9], 0, true).is_ok());
    }

    /// MarvinBlue/426048731's player 1 resigns in its last update: its town centre gives back
    /// the villager queued (the queue's 46 food; the make action made with the queue's payment
    /// gives nothing more) and its population leaves attributes 80 and 81; the make action's end
    /// notifies the centre's unit AI, which stops. The victory check then ends the game:
    /// player 2, the one side left, wins.
    #[test]
    fn a_resignation_cancels_the_queues_and_the_last_side_wins() {
        let mut t = game();
        t.resign(1, false).unwrap();
        let p = &t.players[1];
        assert_eq!((p.game_status, p.resigned, p.dropped), (LOST, true, false));
        assert_eq!(p.resources[0], 46.0);
        assert_eq!((p.attributes[80], p.attributes[81]), (0.0, 0.0));
        let tc = &p.objects[0];
        let production = tc.production.as_ref().unwrap();
        assert!(production.queue.is_empty() && !production.action);
        let ai = tc.ai.as_ref().unwrap();
        assert_eq!((ai.current_action, ai.last_action), (-1, 0x26b));
        assert_eq!(ai.notifies.len(), 1);
        assert_eq!(t.game_state, 0);
        t.victory_check();
        assert_eq!(t.game_state, 1);
        assert_eq!(
            (t.players[1].game_status, t.players[2].game_status),
            (LOST, WON)
        );
    }

    #[test]
    fn a_make_action_the_queue_did_not_pay_for_refunds_its_master_s_costs() {
        let mut t = game();
        if let Some(production) = t.players[1].objects[0].production.as_mut() {
            production.make_paid = false;
        }
        t.resign(1, false).unwrap();
        assert_eq!(t.players[1].resources[0], 92.0);
    }

    #[test]
    fn early_in_the_game_a_resignation_leaves_the_objects_alone() {
        let mut t = game();
        t.world_time = 60_000;
        t.resign(1, true).unwrap();
        assert_eq!((t.players[1].resigned, t.players[1].dropped), (true, true));
        assert_eq!(t.players[1].resources[0], 0.0);
    }

    #[test]
    fn one_side_left_after_a_resignation_ends_a_conquest_game() {
        let mut c = Count::default();
        c.add(1, LOST, false, (0, 0));
        c.add(2, PLAYING, false, (0, 0));
        assert_eq!((c.playing, c.side, c.lost, c.won), (1, 1, 1, 0));
        // two players still playing, enemies: two sides
        let mut c = Count::default();
        c.add(1, PLAYING, false, (0, 0));
        c.add(2, PLAYING, false, (3, 3));
        assert_eq!((c.playing, c.side), (2, -1));
        // allies with the allied victory are one side
        let mut c = Count::default();
        c.add(1, PLAYING, true, (0, 0));
        c.add(2, PLAYING, true, (0, 0));
        assert_eq!((c.playing, c.side), (2, 1));
    }
}
