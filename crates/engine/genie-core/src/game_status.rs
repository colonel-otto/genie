//! The players' game status (`+0x89`: playing, won, lost) and what changes it: RESIGN
//! (`FUN_140d25af0`), the world's victory check after every update (`TRIBE_World` slot 0x68,
//! `FUN_1415c9480`, whose tail `FUN_14152e3d0` decides who won), and the status setter
//! (`WorldPlayer` slot 1, `FUN_141639860`) with a defeated player's stop of its objects
//! (`FUN_141613ed0`): its villagers, fishing boats and explorers stop and stand, its buildings
//! stop and give back their queues, and every unit AI's orders go.
//!
//! 158041: the game over test `FUN_14152f810`, a player's own victory `FUN_14165d720`, the queue's
//! cancel `FUN_1411cff00`, the make action's destructor `FUN_140ace3f0`.

use crate::production::{Making, refund, refund_research};
use crate::rng::Stream;
use crate::turn::{Releaser, Turns};
use crate::unit_ai::{Effect, GenieError};

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
/// The livestock's class, whose unit AI a stop leaves alone (`FUN_14108c050`: raw `-0xe72`).
const LIVESTOCK: i16 = 58;
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
/// The villager's masters, whose two sexes a cancel of five counts as one kind
/// (`FUN_1411d66b0`, `FUN_1411d61e0`).
const VILLAGER_MALE: i16 = 0x53;
const VILLAGER_FEMALE: i16 = 0x125;
/// The game mode whose QUEUE trains one military unit at a time early on (16, the settings'
/// `+0x34`).
const ONE_UNIT_MODE: i32 = 16;
/// The player's attribute that mode reads (6, `+0x30`).
const CURRENT_AGE: i16 = 6;
/// That attribute's bound (`DAT_14330eb8c`).
const ONE_UNIT_AGE: f32 = crate::image::game_status::ONE_UNIT_AGE;

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
    /// Player `i` of `status`, with the allied victory flag (`+0x2d0`, tested for 0) and the
    /// relations both ways to the first player still playing (`relation` 0: allied;
    /// `FUN_141667f70`).
    const fn add(&mut self, i: usize, status: u8, allied_victory: u8, relations: (u8, u8)) {
        match status {
            PLAYING => {
                self.playing += 1;
                if self.first != 0 && self.side != 0 {
                    // another side unless the two count each other's victory as their own
                    if allied_victory == 0 || relations.0 != 0 || relations.1 != 0 {
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
    pub(crate) fn resign(&mut self, player: usize, dropped: bool) -> Result<(), GenieError> {
        let p = self.players.get_mut(player).ok_or(GenieError::unsupported(
            "RESIGN of a player the slice does not have",
        ))?;
        p.dropped = dropped;
        p.resigned = true;
        self.set_status(player, LOST)
    }

    /// The status setter (slot 1, `FUN_141639860`) of player `p`: `FUN_1416768e0` takes the new
    /// status unless the player resigned (a defeat it always takes). A defeat without a
    /// resignation (the world player's defeat check, [`Turns::defeat_check`]) then posts the
    /// interface's event 6 (`FUN_140c898a0`), the local player's comm message
    /// (`FUN_140d3c710`) and, with a comm handler in session (`DAT_143e8d7d0`'s `+0x388`), marks
    /// the player's comm players defeated or dropped in the game object (`+0xb1c`, `+0xb13`)
    /// and the local one's map flags (`FUN_140c915d0`, `FUN_140c915b0`): the lobby's and the
    /// local view's, which a playback has none of. A status that turned won or
    /// lost while the game goes on reveals the map to the player (`FUN_1412847b0`, with no lobby
    /// settings object or its setting `FUN_141300cd0`, as the playback holds it,
    /// [`Spatial::reveal_to`]); the battle royale's is not modelled. A lost player's objects stop
    /// ([`Turns::stop_everything`]) once the game is a minute old; its fealty (`+0x12f8`) would
    /// give its resources and units away while the game goes on, which is not modelled. A
    /// newly lost player's history takes its defeat ([`Turns::defeat_event`]).
    pub(crate) fn set_status(&mut self, p: usize, status: u8) -> Result<(), GenieError> {
        let old = self.players[p].game_status;
        if status != old && (!self.players[p].resigned || status == LOST) {
            self.players[p].game_status = status;
        }
        let now = self.players[p].game_status;
        if now != old && matches!(now, WON | LOST) && !self.game_ended() {
            // the map revealed to the player (FUN_1412847b0 with 0x4000000040000000): every
            // tile of the unified map and the player's masks take the ended bits; the view's
            // refresh (FUN_140c80bb0) is not held
            self.spatial.reveal_to(p);
        }
        if self.settings.game_mode == BATTLE_ROYALE {
            return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
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
    fn own_victory(&mut self, i: usize) -> Result<u8, GenieError> {
        let p = &self.players[i];
        if p.game_status == LOST || p.victory != 2 {
            return Ok(p.game_status);
        }
        if p.allied_victory != 0 {
            return Err(GenieError::unsupported(
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
    fn stop_everything(&mut self, p: usize) -> Result<(), GenieError> {
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
    pub(crate) fn object_stop(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[idx];
        let master = self.master_of(p, unit.life.master_id)?;
        let class = master.class;
        if unit
            .top_action_kind()
            .is_some_and(|k| KEEPING_TYPES.contains(&k))
        {
            return Err(GenieError::unsupported(
                "a stop of an action of type 0x7c or 0x7d",
            ));
        }
        let keeps = self.halt_top_action(p, idx)?;
        if keeps && class == TRADE_CART {
            return Err(GenieError::unsupported(
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
        if class == LIVESTOCK || ai.current_action == KEPT_ACTION || trading {
            return Ok(());
        }
        let mut fx = Vec::new();
        ai.stop_with(false, Some(place), &mut fx)?;
        for e in fx {
            match e {
                Effect::Stop => self.command_stop(p, idx)?,
                _ => return Err(GenieError::unsupported("a stop's effect not modelled")),
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
    fn halt_top_action(&mut self, p: usize, idx: usize) -> Result<bool, GenieError> {
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
            return Err(GenieError::unsupported(
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
            return Err(GenieError::unsupported(
                "a stopped hunter that tends a pasture",
            ));
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
    fn halt_other(&mut self, p: usize, idx: usize, kind: i16) -> Result<bool, GenieError> {
        let by = self.releaser(p, idx);
        let master_id = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master_id)?.standing_sprite;
        match kind {
            crate::gather::GATHER => self.halt_gather(p, idx, standing, by)?,
            crate::build::BUILD => self.halt_build(p, idx, by)?,
            crate::repair::REPAIR => self.halt_repair(p, idx)?,
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
            crate::artifact::ARTIFACT_TYPE => self.halt_artifact(p, idx, by)?,
            _ => {
                return Err(GenieError::unsupported(
                    "a stop of an action whose slot 0x38 is not modelled",
                ));
            }
        }
        Ok(true)
    }

    /// [`Turns::halt_other`] of a Gather: it halts with `standing` (the task's
    /// carrying one while it carries at least 1 of the task's resource); its target and second
    /// target go, released by `by`. `standing` is the master's standing sprite.
    fn halt_gather(
        &mut self,
        p: usize,
        idx: usize,
        standing: i16,
        by: Releaser,
    ) -> Result<(), GenieError> {
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
        Ok(())
    }

    /// [`Turns::halt_other`] of a Build: a counted builder leaves its foundation's builders, the
    /// unit stands, and the foundation and the second target go, released by `by`.
    fn halt_build(&mut self, p: usize, idx: usize, by: Releaser) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let build = unit.build.as_mut().expect("its top action");
        let foundation = build.target;
        let (counted, targets) = build.halt(unit.mover.as_mut())?;
        if counted {
            self.leave_builders(foundation)?;
        }
        self.standing_sprite(p, idx)?;
        for t in targets {
            if t != -1 {
                self.release(t, by)?;
            }
        }
        Ok(())
    }

    /// [`Turns::halt_other`] of a Repair (`FUN_140adaa00`): `set_state(2)`, a counted repairer
    /// out of its target's repairers, the standing sprite; the target stays.
    fn halt_repair(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let repair = unit.repair.as_mut().expect("its top action");
        let target = repair.target;
        if repair.halt(unit.mover.as_mut())? {
            self.leave_builders(target)?;
        }
        self.standing_sprite(p, idx)
    }

    /// [`Turns::halt_other`] of an Artifact: its sub-actions and its target go (released by
    /// `by`, the Enter's first), a walking animal stops, and it stands.
    fn halt_artifact(&mut self, p: usize, idx: usize, by: Releaser) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let artifact = unit.artifact.as_mut().expect("its top action");
        let (gone, released) = artifact.stop();
        if gone.stops {
            unit.mover
                .as_mut()
                .ok_or(GenieError::unsupported(
                    "a walking animal without movement state",
                ))?
                .stop()?;
        }
        for t in [gone.released, released].into_iter().flatten() {
            self.release(t, by)?;
        }
        self.standing_sprite(p, idx)
    }

    /// `FUN_1411cff00`, the cancel of the whole queue of the building at `idx` of player `p`,
    /// entry by entry. A unit entry: each unit gets its refund ([`refund`]) and the tracker its
    /// place back (the counter one less, [`crate::tracker::Tracker::unplace`]); attribute 80
    /// lowers by the population times the count (slot 0xf8), and the tracker's counter by the
    /// count once more. A research entry, for a player with a tech tree: its count's refund
    /// ([`Turns::refund_researches`]). Then the interface's `+0xc02` (the local player's event
    /// 0x7d) is left; the entries go, the production list's action goes
    /// ([`Turns::end_production_action`]), the change counter grows and the total is 0.
    pub(crate) fn cancel_queue(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let Some(production) = self.players[p].objects[idx].production.as_ref() else {
            return Ok(());
        };
        let entries = production.queue.entries().to_vec();
        for e in &entries {
            if e.tech == -1 {
                self.cancel_units(p, e, e.count)?;
                self.players[p].tracker.count_queued(-e.count);
            } else if !self.players[p].techs.is_empty() {
                self.refund_researches(p, idx, e, e.count)?;
            }
        }
        if let Some(production) = self.players[p].objects[idx].production.as_mut() {
            production.queue.count = 0;
        }
        self.end_production_action(p, idx)?;
        if let Some(production) = self.players[p].objects[idx].production.as_mut() {
            production.queue.changes = production.queue.changes.wrapping_add(1);
            production.queue.total = 0;
        }
        Ok(())
    }

    /// `count` units of a unit entry `e` of player `p` cancelled (`FUN_1411cff00`,
    /// `FUN_1411d61e0`): each its refund ([`refund`]) and its tracker place (the counter one
    /// less, [`crate::tracker::Tracker::unplace`]); then attribute 80 lowers by the population
    /// times the count (slot 0xf8 with −population × count).
    fn cancel_units(
        &mut self,
        p: usize,
        e: &crate::production::Entry,
        count: i16,
    ) -> Result<(), GenieError> {
        use crate::production::{QUEUED, population};
        let class = self.tracker_key(p, e.master).class;
        let master = self.masters.get(p).and_then(|m| m.get(e.master)).cloned();
        let owner = &mut self.players[p];
        for _ in 0..count.max(0) {
            if let Some(m) = &master {
                refund(m, &e.costs, owner)?;
            }
            owner.tracker.count_queued(-1);
            owner.tracker.unplace(e.master, class);
        }
        let pop = master.as_ref().map_or(0.0, |m| population(&m.storage));
        owner.add_attribute(QUEUED, -pop * f32::from(count))
    }

    /// `count` researches of a research entry `e` at the building at `idx` of player `p`
    /// cancelled (`FUN_1411cff00`, `FUN_1411d61e0`): the refund of what the entry paid times
    /// the count (`FUN_1413d15e0`, [`refund_research`]; the technology in state 1 with no work
    /// done), the costs the building paid back to its store (`FUN_1411c7a00`: each of the
    /// `.dat`'s cost entries paid by the building, flag 2, its amount once), the technology out of
    /// the building's disabled list (`FUN_1411c5250`) and its researches queued (`+0x58`) lower
    /// by the count.
    fn refund_researches(
        &mut self,
        p: usize,
        idx: usize,
        e: &crate::production::Entry,
        count: i16,
    ) -> Result<(), GenieError> {
        // the entry's technology is one of the tree: RESEARCH queued it, the load read it
        let Some(t) = usize::try_from(e.tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len())
        else {
            return Ok(());
        };
        let owner = &mut self.players[p];
        refund_research(&e.costs, count, owner)?;
        let state = &mut owner.techs[t];
        state.state = 1;
        state.research_done = 0.0;
        let building = &mut owner.objects[idx];
        if let Some(def) = self.techs.get(t) {
            for c in def.costs.iter().filter(|c| c.paid == 2) {
                match building.store.iter_mut().find(|s| s.0 == c.attribute) {
                    Some(s) => s.1 += f32::from(c.amount),
                    None => building
                        .store
                        .push((c.attribute, f32::from(c.amount) + 0.0)),
                }
            }
        }
        if let Some(production) = building.production.as_mut() {
            production.enable_tech(e.tech);
        }
        let pending = &mut owner.techs[t].pending;
        *pending = pending.wrapping_sub(count);
        Ok(())
    }

    /// `UNIT_ORDER`'s action 4 (`FUN_14161e370` case 4 -> `FUN_1411d66b0`) at the building at
    /// `idx` of player `p`: the entry of the `index`-th unit
    /// ([`crate::production::Queue::entry_of_unit`]; none past the last unit) is cancelled.
    /// A unit entry with `all` (the command's `+0x1d`) gives up to five of its kind back
    /// ([`Turns::cancel_five`]); without, one unit ([`Turns::give_back_one`]). A research entry,
    /// for a player with a tech tree: for the first unit the list's first action (the research)
    /// loses its progress (`+0x70`) and the technology its work done; then one research, five
    /// with `all`, goes ([`Turns::cancel_some`] with the entry and its technology). Otherwise
    /// (one unit, or a research entry of a player without a tech tree, which gives nothing
    /// back) the entry's count, the total and the change counter lower, an emptied entry going
    /// ([`crate::production::Queue::close`]), and for the first unit (index 0) the production
    /// list's action goes ([`Turns::end_production_action`]; the interface's `+0xc02` and the
    /// local player's event 0x7d are left).
    pub(crate) fn cancel_queued(
        &mut self,
        p: usize,
        idx: usize,
        index: u32,
        all: bool,
    ) -> Result<(), GenieError> {
        let Some(production) = self.players[p].objects[idx].production.as_ref() else {
            return Ok(());
        };
        let Some(e) = production.queue.entry_of_unit(index) else {
            return Ok(());
        };
        let entry = production.queue.slots[e];
        if entry.tech == -1 {
            if all {
                return self.cancel_five(p, idx, index, e);
            }
            self.give_back_one(p, &entry)?;
        } else if !self.players[p].techs.is_empty() {
            if index == 0 {
                let owner = &mut self.players[p];
                if let Some(Some(action)) = owner.objects[idx]
                    .production
                    .as_mut()
                    .map(|q| q.action.as_mut())
                {
                    match action {
                        Making::Research(r) => r.progress = 0.0,
                        // the game writes the make action's +0x70, its unique id
                        Making::Unit(m) => m.unique_id = 0,
                    }
                    if let Some(state) = usize::try_from(entry.tech)
                        .ok()
                        .and_then(|t| owner.techs.get_mut(t))
                    {
                        state.research_done = 0.0;
                    }
                }
            }
            let count = i16::from(all) * 4 + 1;
            return self.cancel_some(p, idx, (-1, count), e as i32, entry.tech);
        }
        let owner = &mut self.players[p];
        let queue = &mut owner.objects[idx].production.as_mut().expect("above").queue;
        queue.slots[e].count -= 1;
        queue.total -= 1;
        queue.changes = queue.changes.wrapping_add(1);
        if queue.slots[e].count < 1 {
            queue.close(e, true);
        }
        if index == 0 {
            self.end_production_action(p, idx)?;
        }
        Ok(())
    }

    /// `FUN_1411d66b0`'s one unit of entry `entry` given back to player `p`: its refund
    /// ([`refund`]), the tracker's counter one less and its place back, attribute 80 lower by
    /// its population (slot 0xf8).
    fn give_back_one(
        &mut self,
        p: usize,
        entry: &crate::production::Entry,
    ) -> Result<(), GenieError> {
        let class = self.tracker_key(p, entry.master).class;
        let master = self
            .masters
            .get(p)
            .and_then(|m| m.get(entry.master))
            .cloned();
        let owner = &mut self.players[p];
        if let Some(m) = &master {
            refund(m, &entry.costs, owner)?;
        }
        owner.tracker.count_queued(-1);
        owner.tracker.unplace(entry.master, class);
        let pop = master
            .as_ref()
            .map_or(0.0, |m| crate::production::population(&m.storage));
        owner.add_attribute(crate::production::QUEUED, -pop)
    }

    /// `FUN_1411d66b0`'s cancel of up to five of a kind, from the unit entry `e` of the
    /// `index`-th unit of the queue of the building at `idx` of player `p`: while the entry at
    /// `e` is a unit entry with units of the first entry's master or of a villager's (83, 293:
    /// the two sexes count as one kind), at most what is left of five go
    /// ([`Turns::cancel_some`] with its master, from `index`). When the entry at `e` is then a
    /// research's, or `e` is past the last entry, `e` steps back one; the loop ends once five
    /// went, the queue is empty, or the entry at `e` is a research's, empty or of another
    /// kind. A step back from the first entry reads before the buffer in the game (heap memory
    /// the simulation does not hold): taken as no entry of the kind, which ends the loop.
    fn cancel_five(
        &mut self,
        p: usize,
        idx: usize,
        index: u32,
        e: usize,
    ) -> Result<(), GenieError> {
        fn queue(t: &Turns, p: usize, idx: usize) -> &crate::production::Queue {
            &t.players[p].objects[idx]
                .production
                .as_ref()
                .expect("the caller's production")
                .queue
        }
        let first = queue(self, p, idx).slots[e];
        let kind = first.master;
        let mut e = e as i32;
        let mut left = first.count.min(5);
        let mut master = kind;
        let mut gone: u16 = 0;
        let mut n = queue(self, p, idx).count;
        loop {
            if n == 0 {
                return Ok(());
            }
            let Some(slot) = usize::try_from(e)
                .ok()
                .map(|e| queue(self, p, idx).slots[e])
            else {
                return Ok(());
            };
            if slot.tech != -1 || slot.count == 0 {
                return Ok(());
            }
            if master != kind && master != VILLAGER_MALE && master != VILLAGER_FEMALE {
                return Ok(());
            }
            let take = if left <= slot.count { left } else { slot.count };
            self.cancel_some(p, idx, (master, take), index as i32, -1)?;
            gone = gone.wrapping_add(take as u16);
            let q = queue(self, p, idx);
            let at = q.slots[e as usize];
            n = q.count;
            if at.tech != -1 || i32::from(n) - 1 < e {
                e -= 1;
            }
            if 0 < n {
                left = 5 - gone as i16;
                // before the buffer: no master of the kind
                master = usize::try_from(e).map_or(-1, |e| q.slots[e].master);
            } else {
                left = take;
            }
            if 4 < gone as i16 {
                return Ok(());
            }
        }
    }

    /// `FUN_1411d61e0`, the cancel of `count` of master `master` (or of technology `tech`, not
    /// −1) in the queue of the building at `idx` of player `p`, from entry `from` (held to the
    /// last). A research: the entry at `from` gives back what its researches paid
    /// ([`Turns::refund_researches`], its count at most the entry's). A unit: the first entry
    /// at `from` or before it of that master (none: nothing) gives back its units, at most its
    /// count ([`Turns::cancel_units`]), and the tracker's counter lowers by the count once more.
    /// The entry's count, the total and the change counter lower; an emptied entry goes
    /// ([`crate::production::Queue::close`]; a research's leaves the train times in their
    /// slots). For the first entry the production list's action goes
    /// ([`Turns::end_production_action`]; the interface's `+0xc02` and the local player's event
    /// 0x7d are left) unless a villager's entry went, the cancel did not start at the first
    /// unit, and a villager's entry with units followed it (the make action goes on for the
    /// other sex).
    pub(crate) fn cancel_some(
        &mut self,
        p: usize,
        idx: usize,
        (master, count): (i16, i16),
        from: i32,
        tech: i16,
    ) -> Result<(), GenieError> {
        let Some(production) = self.players[p].objects[idx].production.as_ref() else {
            return Ok(());
        };
        let q = &production.queue;
        if q.count == 0 {
            return Ok(());
        }
        let start = from.min(i32::from(q.count) - 1);
        if start < 0 {
            return Ok(());
        }
        let found = if tech == -1 {
            (0..=start as usize)
                .rev()
                .find(|&i| q.slots[i].master == master)
        } else {
            Some(start as usize)
        };
        let Some(i) = found else {
            return Ok(());
        };
        let entry = q.slots[i];
        let take = count.min(entry.count);
        if tech == -1 {
            self.cancel_units(p, &entry, take)?;
            self.players[p].tracker.count_queued(-take);
        } else {
            let e = crate::production::Entry { tech, ..entry };
            self.refund_researches(p, idx, &e, take)?;
        }
        let queue = &mut self.players[p].objects[idx]
            .production
            .as_mut()
            .expect("above")
            .queue;
        queue.slots[i].count = queue.slots[i].count.wrapping_sub(take);
        queue.total = queue.total.wrapping_sub(take);
        queue.changes = queue.changes.wrapping_add(1);
        if 0 < queue.slots[i].count {
            return Ok(());
        }
        let villager = |m: i16| m == VILLAGER_MALE || m == VILLAGER_FEMALE;
        // the game reads the next slot before the move; it matters for the first entry alone
        let pair = tech == -1
            && i == 0
            && villager(queue.slots[0].master)
            && queue.slots.get(1).is_some_and(|s| villager(s.master));
        queue.close(i, tech == -1);
        if i != 0 {
            return Ok(());
        }
        if tech == -1 && start != 0 && pair && 0 < queue.slots[0].count {
            return Ok(());
        }
        self.end_production_action(p, idx)
    }

    /// `FUN_14161e370` case 4's tail after the cancel at the building at `idx` of player `p`:
    /// in game mode 16, with the player's attribute 6 at most 1 (`COMISS`, `JC`: a NaN is not),
    /// the master of the queue's slot at the unit index `index` (read as a slot's index, after
    /// the cancel) keys a military class ([`crate::tracker::Key::military`]): the player's flag
    /// of a military unit queued in that mode (`+0x1378`) clears. An index past the buffer
    /// reads heap memory the simulation does not hold: taken as no master.
    pub(crate) fn cancel_tail(
        &mut self,
        p: usize,
        idx: usize,
        index: u32,
    ) -> Result<(), GenieError> {
        if self.settings.game_mode != ONE_UNIT_MODE {
            return Ok(());
        }
        let attribute = crate::tech::Attributes::get(&self.players[p], CURRENT_AGE)?;
        if crate::fmath::below_or_unordered(ONE_UNIT_AGE, attribute) {
            return Ok(());
        }
        let master = self.players[p].objects[idx]
            .production
            .as_ref()
            .and_then(|q| q.queue.slots.get(index as i32 as usize))
            .map(|s| s.master);
        if let Some(master) = master
            && self.tracker_key(p, master).military
        {
            self.players[p].one_military_queued = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::{Master, Masters};
    use crate::production::{Entry, MakeObj, MakeTech, Making, Production, Queue};
    use crate::turn::{Player, Unit};
    use crate::unit_ai::UnitAi;

    /// An entry of `count` villagers of master `master` that paid `food` each.
    fn villager(master: i16, count: i16, food: f32) -> Entry {
        Entry {
            master,
            count,
            tech: -1,
            costs: [(0, food), (-1, 0.0), (4, 1.0)],
            serial: 0,
            button: -1,
            train_time: 25,
        }
    }

    /// A queue of ten slots holding `entries`.
    fn queue(entries: &[Entry]) -> Queue {
        let mut slots = vec![Entry::default(); 10];
        slots[..entries.len()].copy_from_slice(entries);
        Queue {
            slots,
            count: entries.len() as i16,
            total: entries.iter().map(|e| e.count).sum(),
            next_serial: 10,
            ..Queue::default()
        }
    }

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
                queue: queue(&[villager(83, 1, 46.0)]),
                action: Some(Making::Unit(MakeObj {
                    state: MakeObj::MAKING,
                    progress: 3.0,
                    ..MakeObj::for_entry(&villager(83, 1, 46.0))
                })),
                ..Production::default()
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

    /// Slot 0x38 of the actions with their own ([`Turns::halt_other`]), each true: the Gather
    /// idles (state 2), the Build and the Repair halt (state 2), the Attack ends (state 1, its
    /// animation at speed 1, standing) and the Artifact checks again (state 6); a kind it does
    /// not model is refused.
    #[test]
    fn each_action_with_its_own_stop_halts_and_answers_true() {
        let mut t = game();
        let mut add = |unit: Unit| {
            let mut unit = unit;
            unit.life.master_id = 109;
            t.players[1].objects.push(unit);
            t.players[1].objects.len() - 1
        };
        let task = crate::hunt::HuntTask {
            id: 0,
            range: 0.25,
            resource: 17,
            sprites: [-1; 4],
            work_value: 1.0,
            productivity: -1,
            kind4: -1,
        };
        let bush = crate::gather::Resource {
            id: -1,
            position: [3.5, 3.5, 0.0],
            class: 7,
            master: 59,
            player: 0,
            workers: 0,
            drop_site: false,
            gap: 1.2,
            radius: [0.5, 0.5],
        };
        let rate = crate::gather::StopRate {
            work_rate: 1.0,
            capacity: 10.0,
            productivity: 1.0,
        };
        let gather = add(Unit {
            id: 30,
            gather: Some(crate::gather::GatherAction::new(task, &bush, false, rate).unwrap()),
            ..Unit::default()
        });
        let build_task = crate::build::BuildTask {
            range: 0.0,
            work_value: 1.0,
            work_rate: 1.0,
            resource: -1,
            id: -1,
        };
        let build = add(Unit {
            id: 31,
            build: Some(crate::build::BuildAction::start(
                -1,
                [3.5, 3.5, 0.0],
                1.0,
                build_task,
            )),
            ..Unit::default()
        });
        let repair_task = crate::repair::RepairTask {
            id: 0,
            work_value: 1.0,
        };
        let repair = add(Unit {
            id: 32,
            repair: Some(crate::repair::RepairAction::start(
                -1,
                [3.5, 3.5, 0.0],
                repair_task,
                0.0,
            )),
            ..Unit::default()
        });
        let mut attack = crate::attack::tests::own_attack(-1, crate::attack::DONE, 1.0);
        attack.arms.idle_sprite = Some(crate::sprite::Graphic {
            id: 1,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 0,
        });
        let attack = add(Unit {
            id: 33,
            attack: Some(attack),
            ..Unit::default()
        });
        let artifact = add(Unit {
            id: 34,
            artifact: Some(crate::artifact::Artifact::new(false)),
            ..Unit::default()
        });
        assert_eq!(t.halt_other(1, gather, crate::gather::GATHER), Ok(true));
        assert_eq!(t.halt_other(1, build, crate::build::BUILD), Ok(true));
        assert_eq!(t.halt_other(1, repair, crate::repair::REPAIR), Ok(true));
        assert_eq!(t.halt_other(1, attack, crate::attack::ATTACK), Ok(true));
        let kind = crate::artifact::ARTIFACT_TYPE;
        assert_eq!(t.halt_other(1, artifact, kind), Ok(true));
        let objects = &t.players[1].objects;
        assert_eq!(objects[gather].gather.map(|g| g.state), Some(2));
        assert_eq!(objects[build].build.map(|b| b.state), Some(2));
        assert_eq!(objects[repair].repair.map(|r| r.state), Some(2));
        let a = &objects[attack];
        assert_eq!(a.attack.map(|a| a.state), Some(crate::attack::DONE));
        assert_eq!(a.anim_speed, Some(1.0));
        assert!(a.sprite.is_some());
        assert_eq!(objects[artifact].artifact.map(|a| a.state), Some(6));
        assert!(t.halt_other(1, gather, 99).is_err());
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
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[villager(83, 1, 46.0), villager(83, 1, 45.0)]);
        t.players[1].attributes[80] = 2.0;
        t.cancel_order(&[9], 1, false).unwrap();
        let p = &t.players[1];
        assert_eq!((p.resources[0], p.attributes[80]), (45.0, 1.0));
        let production = p.objects[0].production.as_ref().unwrap();
        let q = &production.queue;
        assert_eq!((q.count, q.total, q.changes), (1, 1, 1));
        assert!(production.action.is_some());
        assert_eq!(p.tracker.queued, -1);
        t.cancel_order(&[9], 5, false).unwrap();
        assert_eq!(t.players[1].resources[0], 45.0);
        t.cancel_order(&[9], 0, false).unwrap();
        let p = &t.players[1];
        assert_eq!(
            (p.resources[0], p.attributes[80], p.attributes[81]),
            (91.0, 0.0, 0.0)
        );
        let production = p.objects[0].production.as_ref().unwrap();
        assert!(production.queue.entries().is_empty() && production.action.is_none());
        assert!(t.cancel_order(&[9], 0, true).is_ok());
    }

    /// One unit of an entry of three goes: the entry keeps two, and the first unit's cancel
    /// drops the make action though the entry stays (`FUN_1411d66b0`).
    #[test]
    fn a_cancel_of_the_first_unit_drops_the_make_action_of_an_entry_that_stays() {
        let mut t = game();
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[villager(83, 3, 46.0)]);
        t.players[1].attributes[80] = 3.0;
        t.cancel_queued(1, 0, 0, false).unwrap();
        let p = &t.players[1];
        let production = p.objects[0].production.as_ref().unwrap();
        assert_eq!(
            (production.queue.count, production.queue.slots[0].count),
            (1, 2)
        );
        assert!(production.action.is_none());
        assert_eq!((p.attributes[80], p.attributes[81]), (2.0, 0.0));
    }

    /// `FUN_1411d66b0` with `all` on the first villager of [3 male, 4 female]: the three go
    /// (`FUN_1411d61e0`), the first entry with them, and the make action with it (the cancel
    /// started at unit 0); the female entry moved to the first slot gives two more, five in
    /// all. Each unit gives its 46 food back and leaves attribute 80; the tracker's counter
    /// loses two per unit (the game counts a cancel of several twice).
    #[test]
    fn a_cancel_of_all_takes_five_villagers_of_both_sexes() {
        let mut t = game();
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[villager(83, 3, 46.0), villager(293, 4, 46.0)]);
        t.players[1].attributes[80] = 7.0;
        t.masters[1] = Masters::new([
            t.master_of(1, 109).unwrap().clone(),
            t.master_of(1, 83).unwrap().clone(),
            Master {
                id: 293,
                ..t.master_of(1, 83).unwrap().clone()
            },
        ]);
        t.cancel_order(&[9], 0, true).unwrap();
        let p = &t.players[1];
        assert_eq!((p.resources[0], p.attributes[80]), (5.0 * 46.0, 2.0));
        let production = p.objects[0].production.as_ref().unwrap();
        let q = &production.queue;
        assert_eq!(
            (q.count, q.total, q.slots[0].master, q.slots[0].count),
            (1, 2, 293, 2)
        );
        assert_eq!(q.changes, 2);
        assert!(production.action.is_none());
        assert_eq!(p.tracker.queued, -10);
        // from the second unit the make action goes on for the other sex
        let mut t2 = game();
        t2.masters = t.masters.clone();
        let tc = t2.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[villager(83, 3, 46.0), villager(293, 4, 46.0)]);
        t2.players[1].attributes[80] = 7.0;
        t2.cancel_order(&[9], 1, true).unwrap();
        let production = t2.players[1].objects[0].production.as_ref().unwrap();
        assert_eq!(
            (production.queue.count, production.queue.slots[0].count),
            (1, 2)
        );
        assert!(production.action.is_some());
    }

    /// The kind is the first entry's master or a villager's: all of two archers takes the
    /// villagers after them too; a research shifted into the slot ends the loop.
    #[test]
    fn a_cancel_of_all_goes_on_into_villagers_and_stops_at_a_research() {
        let archer = |count| Entry {
            master: 4,
            costs: [(1, 25.0), (3, 45.0), (4, 1.0)],
            ..villager(4, count, 0.0)
        };
        let mut t = game();
        let ms = &mut t.masters[1];
        *ms = Masters::new(ms.iter().cloned().chain([Master {
            id: 4,
            kind: 70,
            cost: vec![(1, 25, true), (3, 45, true), (4, 1, false)],
            storage: vec![(4, -1.0, 2)],
            ..Master::default()
        }]));
        t.players[1].attributes[80] = 4.0;
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[archer(2), villager(83, 2, 46.0)]);
        t.cancel_queued(1, 0, 0, true).unwrap();
        let p = &t.players[1];
        assert_eq!(p.resources, [92.0, 50.0, 0.0, 90.0]);
        assert_eq!(p.attributes[80], 0.0);
        assert_eq!(p.objects[0].production.as_ref().unwrap().queue.count, 0);
        // [2 villagers, a research]: the research moved into the slot ends it
        let mut t = game();
        let research = Entry {
            master: -1,
            tech: 22,
            costs: [(3, 50.0), (-1, 0.0), (-1, 0.0)],
            ..villager(-1, 1, 0.0)
        };
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[villager(83, 2, 46.0), research]);
        t.players[1].attributes[80] = 2.0;
        t.cancel_queued(1, 0, 0, true).unwrap();
        let q = &t.players[1].objects[0].production.as_ref().unwrap().queue;
        assert_eq!(
            (q.count, q.slots[0].tech, t.players[1].resources[0]),
            (1, 22, 92.0)
        );
    }

    /// Player 1's town centre researching technology 22 three times over (one entry), the
    /// research running with `progress` done: each paid 50 gold, and the `.dat` lists 20 wood
    /// the building pays (flag 2).
    fn researching(progress: f32) -> Turns {
        use crate::tech::{Cost, Location, TechDef, TechState};
        let mut t = game();
        let entry = Entry {
            master: -1,
            tech: 22,
            costs: [(3, 50.0), (-1, 0.0), (-1, 0.0)],
            ..villager(-1, 3, 0.0)
        };
        let tc = t.players[1].objects[0].production.as_mut().unwrap();
        tc.queue = queue(&[entry]);
        tc.action = Some(Making::Research(MakeTech {
            state: MakeObj::MAKING,
            paid: true,
            tech: 22,
            master: -1,
            progress,
        }));
        tc.disabled_techs = vec![22, 5];
        let mut techs = vec![TechState::default(); 23];
        techs[22] = TechState {
            state: 2,
            allow_multiple: true,
            pending: 3,
            research_done: progress,
            research_cap: -1,
            overrides: [(-1, -1, -2); 3],
            locations: vec![Location {
                building: 109,
                time: 25,
                button: 1,
                adjustment: 0.0,
            }],
            ..TechState::default()
        };
        t.players[1].techs = techs;
        let mut defs = vec![TechDef::default(); 23];
        let cost = |attribute, amount, paid| Cost {
            attribute,
            amount,
            paid,
        };
        defs[22] = TechDef {
            costs: [cost(3, 50, 1), cost(1, 20, 2), cost(-1, 0, 0)],
            locations: vec![(109, 25)],
            ..TechDef::default()
        };
        t.techs = defs;
        t
    }

    /// `FUN_1411d66b0` on the first research with `all`: the running research loses its
    /// progress and the technology its work done; `FUN_1411d61e0` with five takes the entry's
    /// three: 150 gold back (`FUN_1413d15e0`), the technology in state 1 with none queued, the
    /// 20 wood the building paid to its store (`FUN_1411c7a00`), 22 off its disabled list
    /// (`FUN_1411c5250`); the emptied first entry drops the research, whose destructor tells
    /// the centre's unit AI it failed.
    #[test]
    fn a_cancel_of_all_researches_gives_back_the_entry_s_three() {
        let mut t = researching(10.0);
        t.cancel_order(&[9], 0, true).unwrap();
        let p = &t.players[1];
        assert_eq!(p.resources[3], 150.0);
        let s = &p.techs[22];
        assert_eq!((s.state, s.pending, s.research_done), (1, 0, 0.0));
        let tc = &p.objects[0];
        assert_eq!(tc.store, [(1, 20.0)]);
        let production = tc.production.as_ref().unwrap();
        assert_eq!(production.disabled_techs, [5]);
        assert_eq!((production.queue.count, production.queue.total), (0, 0));
        assert!(production.action.is_none());
        assert_eq!(tc.ai.as_ref().unwrap().notifies.len(), 1);
        // one research of the second unit: 50 gold, the technology in state 1 though two stay
        // queued, the research going on
        let mut t = researching(10.0);
        t.cancel_order(&[9], 1, false).unwrap();
        let p = &t.players[1];
        assert_eq!(
            (p.resources[3], p.techs[22].state, p.techs[22].pending),
            (50.0, 1, 2)
        );
        let production = p.objects[0].production.as_ref().unwrap();
        assert_eq!(production.queue.slots[0].count, 2);
        assert!(production.action.is_some());
    }

    /// `FUN_1411cff00` gives a research entry back as a whole; the research's destructor
    /// (`FUN_140ad0be0`) then counts a stacking technology whose progress reached its time as
    /// researched once more and queued once less, in state 1 below its cap.
    #[test]
    fn a_queue_s_cancel_gives_back_its_researches_and_the_destructor_counts_a_done_one() {
        let mut t = researching(30.0);
        let s = &mut t.players[1].techs[22];
        (s.stack, s.research_cap) = (true, 5);
        t.cancel_queue(1, 0).unwrap();
        let p = &t.players[1];
        assert_eq!(p.resources[3], 150.0);
        let s = &p.techs[22];
        assert_eq!(
            (s.state, s.times_researched, s.pending, s.research_done),
            (1, 1, -1, 30.0)
        );
        let production = p.objects[0].production.as_ref().unwrap();
        assert_eq!((production.queue.count, production.queue.changes), (0, 1));
        // not done: state 1, nothing counted
        let mut t = researching(10.0);
        let s = &mut t.players[1].techs[22];
        (s.stack, s.research_cap) = (true, 5);
        t.cancel_queue(1, 0).unwrap();
        let s = &t.players[1].techs[22];
        assert_eq!((s.state, s.times_researched, s.pending), (1, 0, 0));
    }

    /// `0x140ad2a20` after a research completes: attribute 269 (the Spanish's 300, cut by
    /// `CVTTSS2SI`, its low short above 0) names the effect of the world's table (`+0x128`, slot
    /// 0x10 `FUN_141462440`) the player gets with the building as its object: effect 300 adds 20
    /// gold; an id the table does not hold does nothing.
    #[test]
    fn a_research_gives_the_effect_attribute_269_names() {
        use crate::tech::EffectCommand;
        let run = |reward: f32| {
            let mut t = researching(25.0);
            t.players[1].attributes.resize(300, 0.0);
            t.players[1].attributes[269] = reward;
            t.effects = vec![Vec::new(); 301];
            t.effects[300] = vec![EffectCommand {
                kind: 1,
                a: 3,
                b: 1,
                c: -1,
                d: 20.0,
            }];
            let gold = t.players[1].resources[3];
            t.produce(1, 0, 1.0).unwrap();
            t.players[1].resources[3] - gold
        };
        assert_eq!(run(300.0), 20.0);
        assert_eq!(run(300.9), 20.0);
        assert_eq!(run(0.0), 0.0);
        assert_eq!(run(301.0), 0.0);
        // the short of 65836 is 300
        assert_eq!(run(65_836.0), 20.0);
    }

    /// Game mode 16's tail: with attribute 6 at most 1 a cancel whose slot at the unit index
    /// holds a military master clears the player's flag (`+0x1378`); above 1 it stays.
    #[test]
    fn a_cancel_in_game_mode_16_clears_the_military_flag() {
        let run = |age: f32| {
            let mut t = game();
            t.settings.game_mode = 16;
            t.players[1].attributes[6] = age;
            t.players[1].one_military_queued = true;
            let ms = &mut t.masters[1];
            *ms = Masters::new(ms.iter().cloned().chain([Master {
                id: 4,
                class: 6,
                kind: 70,
                ..Master::default()
            }]));
            let tc = t.players[1].objects[0].production.as_mut().unwrap();
            tc.queue = queue(&[villager(83, 1, 46.0), villager(4, 1, 0.0)]);
            t.cancel_order(&[9], 1, false).unwrap();
            t.players[1].one_military_queued
        };
        assert!(!run(1.0));
        assert!(run(2.0));
        assert!(run(f32::NAN));
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
        assert!(production.queue.entries().is_empty() && production.action.is_none());
        assert_eq!((production.queue.total, production.queue.changes), (0, 1));
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
        if let Some(Some(Making::Unit(make))) = t.players[1].objects[0]
            .production
            .as_mut()
            .map(|q| q.action.as_mut())
        {
            make.paid = false;
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
        c.add(1, LOST, 0, (0, 0));
        c.add(2, PLAYING, 0, (0, 0));
        assert_eq!((c.playing, c.side, c.lost, c.won), (1, 1, 1, 0));
        // two players still playing, enemies: two sides
        let mut c = Count::default();
        c.add(1, PLAYING, 0, (0, 0));
        c.add(2, PLAYING, 0, (3, 3));
        assert_eq!((c.playing, c.side), (2, -1));
        // allies with the allied victory are one side
        let mut c = Count::default();
        c.add(1, PLAYING, 1, (0, 0));
        c.add(2, PLAYING, 1, (0, 0));
        assert_eq!((c.playing, c.side), (2, 1));
    }
}
