//! A player's diplomacy change (the world player's slot 0x30, `FUN_141639690`, which GAME's
//! sub-command 0 runs): its stance toward another player (`diplomacy[i]`, `+0xe12`), the
//! relation it makes (`relation[i]`, `+0x260`) and what its queries make of that player's objects
//! (`unit_diplomacy[i]`, `+0x278`); its objects hear of it (slot 0x120: units inside a building or
//! siege tower of a player no longer allied leave it); the players' sight masks and tile lists
//! are made again; then the world's teams (`FUN_1415e23f0`) and the units the players reveal
//! (`FUN_14152bcb0`).
//!
//! 158041: slot 0x30 `FUN_141639690`; the stance `FUN_1416397d0`; the relation `FUN_141676b80`;
//! slot 0x120 of a combat object `FUN_141217120` and of a building `FUN_1411d50e0`; the teams
//! `FUN_1415e23f0`; the revealed units `FUN_14152bcb0`; the tile list's refill `FUN_141285cb0`.

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::masters::master_type;
use crate::turn::Turns;

/// The reveal of a player's units of a kind to the others (its attribute 204 not 0,
/// `FUN_14152bcb0`): their per-player marks (`+0x798`, `+0x79c`) and flares are not modelled.
pub(crate) const REVEAL_UNITS: GenieError =
    GenieError::unsupported("a player's attribute 204 (FUN_14152bcb0)");

/// The attribute naming the units a player reveals to the others (204, `+0x660`).
const REVEAL_UNIT: usize = 204;
/// The market (master 84), whose owner's diplomacy change toward a computer player shows it to
/// that player's tile list.
const MARKET: i16 = 84;
/// The siege tower's class (13), whose units leave it for a player no longer allied.
const SIEGE_CLASS: i16 = 13;
/// The relation of an ally (0) and of a neutral (1); the stances an ally (4) and a neutral (3)
/// take, and an enemy's (2).
const ALLY: u8 = 0;
const NEUTRAL: u8 = 1;
const ALLY_STANCE: u8 = 4;
const NEUTRAL_STANCE: u8 = 3;
const ENEMY_STANCE: u8 = 2;
/// What a player's queries make of another's objects (`+0x278`): Gaia's, its own, an ally's, a
/// neutral's, an enemy's.
const GAIA_KIND: i32 = 0;
const OWN_KIND: i32 = 1;
const ALLY_KIND: i32 = 2;
const NEUTRAL_KIND: i32 = 3;
/// The teams a world has at most past the lone one (2 to 5, `FUN_1415e23f0`).
const LAST_TEAM: i32 = 5;
/// The bits of the unified map a player's masks name players by (`FUN_141285cb0`'s
/// `0x1ffffff01ffffff`).
const PLAYER_BITS: u64 = 0x01ff_ffff_01ff_ffff;

/// `FUN_1416397d0`'s relation for a stance: 1 and 2 an enemy's (3; 4 with the world's
/// `+0x820` object in its state 1, `FUN_1415e8e10`, which the slice does not hold, as the task
/// finder's [`crate::tasks::allied`]), 3 a neutral's, 4 and 5 an ally's; another none.
#[must_use]
pub const fn relation_of_stance(stance: u8) -> Option<u8> {
    match stance {
        1 | 2 => Some(3),
        3 => Some(NEUTRAL),
        4 | 5 => Some(ALLY),
        _ => None,
    }
}

/// `FUN_141676b80`'s kind of player `q`'s objects for player `p` of relation `relation` toward
/// it: Gaia's 0, its own 1, relations 0 and 4 an ally's (2), 1 a neutral's (3), any other an
/// enemy's (4).
#[must_use]
pub const fn unit_kind(p: usize, q: usize, relation: u8) -> i32 {
    if q == 0 {
        GAIA_KIND
    } else if p == q {
        OWN_KIND
    } else if relation & 0xfb == 0 {
        ALLY_KIND
    } else if relation == NEUTRAL {
        NEUTRAL_KIND
    } else {
        NEUTRAL_KIND + 1
    }
}

impl Turns {
    /// Slot 0x30 (`FUN_141639690`) of player `p` toward player `other` with the command's
    /// relation `relation`: 0 makes the stance an ally's (4), 1 a neutral's (3), 3 and 4 an
    /// enemy's (2); another does nothing, as a player outside the world. A stance other than the
    /// one held (`+0xe12` at the other's number) is set ([`Turns::set_stance`]); the interface's
    /// event 8 aside, the world's sight masks are made again (its slot 0x78 with 0,
    /// [`Turns::rebuild_vision`]), both players' tile lists empty (`FUN_141661030`) and refill
    /// with the tiles their masks reach ([`Turns::refill_tiles`]). Then, whether or not it
    /// changed, the world's teams ([`Turns::resolve_teams`]) and the revealed units
    /// ([`Turns::reveal_units`]).
    pub(crate) fn set_diplomacy(
        &mut self,
        p: usize,
        other: i32,
        relation: u32,
    ) -> Result<(), GenieError> {
        let stance = match relation {
            0 => ALLY_STANCE,
            1 => NEUTRAL_STANCE,
            3 | 4 => ENEMY_STANCE,
            _ => return Ok(()),
        };
        let Some(q) = usize::try_from(other)
            .ok()
            .filter(|&q| q < self.players.len())
        else {
            return Ok(());
        };
        if self.players[p].diplomacy.get(q).copied().unwrap_or(0) != stance {
            self.set_stance(p, q, stance)?;
            self.rebuild_vision()?;
            if let Some(v) = self.spatial.sight.as_mut() {
                v.clear_new_tiles(p);
                v.clear_new_tiles(q);
            }
            self.refill_tiles(p);
            self.refill_tiles(q);
        }
        self.resolve_teams();
        self.reveal_units()
    }

    /// `FUN_1416397d0`: player `p`'s stance toward `q` takes `stance`; one that makes a relation
    /// ([`relation_of_stance`]) sets it ([`Turns::set_relation`]).
    pub(crate) fn set_stance(&mut self, p: usize, q: usize, stance: u8) -> Result<(), GenieError> {
        let diplomacy = &mut self.players[p].diplomacy;
        if diplomacy.len() <= q {
            diplomacy.resize(q + 1, 0);
        }
        diplomacy[q] = stance;
        relation_of_stance(stance).map_or(Ok(()), |relation| self.set_relation(p, q, relation))
    }

    /// `FUN_141676b80`: player `p`'s relation toward `q` (`+0x260`, which the walk's and the
    /// queries' copies follow) and what its queries make of `q`'s objects (`+0x278`,
    /// [`unit_kind`]); then each of its live objects, in the list's order, hears of it (slot
    /// 0x120, [`Turns::notify_of_relation`]; the list's count is read again after each).
    pub(crate) fn set_relation(
        &mut self,
        p: usize,
        q: usize,
        relation: u8,
    ) -> Result<(), GenieError> {
        let set = |row: &mut Vec<u8>| {
            if row.len() <= q {
                row.resize(q + 1, 0);
            }
            row[q] = relation;
        };
        set(&mut self.players[p].relations);
        if let Some(row) = self.spatial.relations.get_mut(p) {
            set(row);
        }
        if let Some(row) = self.spatial.unit_diplomacy.get_mut(p) {
            if row.len() <= q {
                row.resize(q + 1, 0);
            }
            row[q] = unit_kind(p, q, relation);
        }
        let mut i = 0;
        while i < self.players[p].objects.len() {
            let id = self.players[p].objects[i].id;
            self.notify_of_relation(p, id, q, relation)?;
            i += 1;
        }
        Ok(())
    }

    /// Slot 0x120 of player `p`'s object `id`, told its player's relation toward `q` is now
    /// `relation`, by its master's type: a combat object's (type 70, `FUN_141217120`) and a
    /// building's (type 80, `FUN_1411d50e0`: the combat object's first); any other's does
    /// nothing (`FUN_141196990`, `FUN_14106d4e0`).
    ///
    /// The combat object's, with a relation other than an ally's: a unit inside an object of
    /// `q` leaves it, a building's toward no point (`FUN_1411e7220` with −1, −1,
    /// [`Turns::let_out`]), a siege tower's (class 13) as its unload; then a siege tower lets out
    /// the units of `q` inside it, from the last. One that finds no spot would leave it and be
    /// destroyed (slots 0x1a8 and 0xd0), which the unload's own errors stand for.
    ///
    /// The building's, with an ally's relation: a market (84) of a player not a computer's tells
    /// a computer player `q` of its tile (its tile list, `FUN_14165d500`), unless the lobby's
    /// rule table (`FUN_14152fbb0`'s, key 1) says not; the table's settings of a scenario are not
    /// held (taken as absent, the rule on). With another relation the units of `q` inside it,
    /// from the last, leave it toward no point.
    pub(crate) fn notify_of_relation(
        &mut self,
        p: usize,
        id: i32,
        q: usize,
        relation: u8,
    ) -> Result<(), GenieError> {
        let Some((_, u)) = self.object(id) else {
            return Ok(());
        };
        let master = self.master_of(p, u.life.master_id)?;
        let (kind, class, master_id) = (master.kind, master.class, master.id);
        if kind != master_type::COMBAT && kind != master_type::BUILDING {
            return Ok(());
        }
        let inside = u.life.inside;
        let (position, garrison) = (u.position, u.garrison.clone());
        if relation != ALLY {
            if let Some(holder) = inside
                && let Some((hp, h)) = self.object(holder)
                && hp == q
            {
                let holder_master = self.master_of(hp, h.life.master_id)?;
                if holder_master.kind == master_type::BUILDING {
                    let hi = self.live_index(hp, holder)?;
                    if !self.let_out(hp, hi, id, [NO_POINT; 2], (None, true))? {
                        self.destroy(p, id)?;
                    }
                } else if holder_master.class == SIEGE_CLASS {
                    return Err(crate::garrison::SIEGE_UNLOAD);
                }
            }
            if class == SIEGE_CLASS
                && garrison
                    .iter()
                    .any(|&g| self.object(g).is_some_and(|(gp, _)| gp == q))
            {
                return Err(crate::garrison::SIEGE_UNLOAD);
            }
        }
        if kind != master_type::BUILDING {
            return Ok(());
        }
        if relation == ALLY {
            if master_id == MARKET
                && !self.players[p].computer
                && i32::try_from(q).ok() != Some(id)
                && self.players[q].computer
                && let Some(v) = self.spatial.sight.as_mut()
            {
                let tile = [cvttss2si(position[0]), cvttss2si(position[1])];
                if let Some(list) = v.new_tiles.get_mut(q) {
                    list.push(tile);
                }
            }
            return Ok(());
        }
        let bi = self.live_index(p, id)?;
        for &g in garrison.iter().rev() {
            if self.object(g).is_some_and(|(gp, _)| gp == q)
                && !self.let_out(p, bi, g, [NO_POINT; 2], (None, true))?
            {
                self.destroy(q, g)?;
            }
        }
        Ok(())
    }

    /// The index of object `id` in player `p`'s live list.
    fn live_index(&self, p: usize, id: i32) -> Result<usize, GenieError> {
        self.players[p]
            .objects
            .iter()
            .position(|u| u.id == id)
            .ok_or(GenieError::unsupported(
                "a unit inside a building outside the live lists",
            ))
    }

    /// `FUN_141285cb0` on player `p`'s map: each tile of the map (rows, then columns) whose
    /// unified word has a player bit of the player's visible or explored mask joins its tile
    /// list (`FUN_14165d500`).
    pub(crate) fn refill_tiles(&mut self, p: usize) {
        let [visible, explored] = self.spatial.vision(p);
        if let Some(sight) = self.spatial.sight.as_mut() {
            let (w, h) = (sight.width, sight.height);
            for y in 0..h {
                for x in 0..w {
                    let word = sight.unified[(y * w + x) as usize];
                    let seen =
                        (visible & word & PLAYER_BITS) != 0 || (explored & word & PLAYER_BITS) != 0;
                    if seen && let Some(list) = sight.new_tiles.get_mut(p) {
                        list.push([x, y]);
                    }
                }
            }
        }
    }

    /// `FUN_1415e23f0`: the world's teams again, unless the options lock them (`mTeamsLocked`,
    /// [`crate::settings::Settings::teams_locked`]). Every player's team is 1 (alone); then each
    /// player past Gaia still alone, while teams 2 to 5 are left, takes into the next team every
    /// other player still alone it and who count each other as allies (relation 0 both ways) and
    /// who both have allied victory at 1 (`FUN_1416625a0`); with any taken, it joins that team
    /// too and the next team is one more (past 5 none is left).
    pub fn resolve_teams(&mut self) {
        if self.settings.teams_locked {
            return;
        }
        let n = self.players.len();
        self.teams = [1; 8];
        let mut team = 2;
        for i in 1..n {
            if 2 <= self.team(i) || team == 1 {
                continue;
            }
            let mut joined = 0;
            for j in 1..n {
                let both = |a: usize, b: usize| {
                    self.players[a].relations.get(b) == Some(&ALLY)
                        && self.players[a].allied_victory == 1
                };
                if j != i && self.team(j) < 2 && both(i, j) && both(j, i) {
                    joined += 1;
                    self.set_team(j, team);
                }
            }
            if 0 < joined {
                self.set_team(i, team);
                team += 1;
                if LAST_TEAM < team {
                    team = 1;
                }
            }
        }
    }

    /// Player `i`'s team (`+0x984` at `i − 1`), 1 for a player past the eight.
    fn team(&self, i: usize) -> i32 {
        self.teams.get(i - 1).copied().unwrap_or(1)
    }

    fn set_team(&mut self, i: usize, team: i32) {
        if let Some(t) = self.teams.get_mut(i - 1) {
            *t = team;
        }
    }

    /// `FUN_14152bcb0`: outside the editor's screen (mode 4, which a playback is not) each
    /// player with attribute 204 (`+0x660`, truncated, `CVTTSS2SI`) not 0 reveals its units of
    /// that kind to the others: not modelled ([`REVEAL_UNITS`]); without one nothing happens.
    pub(crate) fn reveal_units(&self) -> Result<(), GenieError> {
        let reveals = self
            .players
            .iter()
            .any(|q| REVEAL_UNIT < q.attributes.len() && cvttss2si(q.attributes[REVEAL_UNIT]) != 0);
        if reveals {
            return Err(REVEAL_UNITS);
        }
        Ok(())
    }
}

/// The point of an unload toward none (`DAT_14330ec20`, the unload's own role).
const NO_POINT: f32 = crate::math::image::diplomacy::NO_POINT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::Player;

    /// Gaia and three players, all enemies, with the walk's and the queries' copies.
    fn world() -> Turns {
        let player = |p: usize| Player {
            attributes: vec![0.0; 300],
            relations: (0..4).map(|q| if q == p { 0 } else { 3 }).collect(),
            diplomacy: vec![0; 9],
            ..Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0; 4], (0..4).map(player).collect());
        t.spatial.relations = t.players.iter().map(|p| p.relations.clone()).collect();
        t.spatial.unit_diplomacy = (0..4)
            .map(|p| (0..4).map(|q| unit_kind(p, q, 3)).collect())
            .collect();
        t
    }

    /// `FUN_141676b80`'s kinds: Gaia's, its own, relations 0 and 4 an ally's, 1 a neutral's,
    /// 2 and 3 an enemy's.
    #[test]
    fn the_queries_kind_follows_the_relation() {
        assert_eq!(unit_kind(1, 0, 0), 0);
        assert_eq!(unit_kind(1, 1, 3), 1);
        assert_eq!(unit_kind(1, 2, 0), 2);
        assert_eq!(unit_kind(1, 2, 4), 2);
        assert_eq!(unit_kind(1, 2, 1), 3);
        assert_eq!(unit_kind(1, 2, 2), 4);
        assert_eq!(unit_kind(1, 2, 3), 4);
    }

    /// `FUN_1416397d0`'s relations: 1, 2 an enemy's, 3 a neutral's, 4, 5 an ally's, others none.
    #[test]
    fn a_stance_makes_its_relation() {
        assert_eq!(relation_of_stance(0), None);
        assert_eq!(relation_of_stance(1), Some(3));
        assert_eq!(relation_of_stance(2), Some(3));
        assert_eq!(relation_of_stance(3), Some(1));
        assert_eq!(relation_of_stance(4), Some(0));
        assert_eq!(relation_of_stance(5), Some(0));
        assert_eq!(relation_of_stance(6), None);
    }

    /// Slot 0x30: relation 0 makes player 1 an ally of 2 one way, in its stance, its relation
    /// and the walk's and queries' copies; 4 makes it an enemy again; 2 and a player outside
    /// the world change nothing.
    #[test]
    fn a_diplomacy_change_sets_the_stance_and_the_relation() {
        let mut t = world();
        t.set_diplomacy(1, 2, 0).unwrap();
        assert_eq!(t.players[1].diplomacy[2], ALLY_STANCE);
        assert_eq!(t.players[1].relations[2], 0);
        assert_eq!(t.players[2].relations[1], 3);
        assert_eq!(t.spatial.relations[1][2], 0);
        assert_eq!(t.spatial.unit_diplomacy[1][2], ALLY_KIND);
        t.set_diplomacy(1, 2, 1).unwrap();
        assert_eq!(t.players[1].diplomacy[2], NEUTRAL_STANCE);
        assert_eq!(t.spatial.unit_diplomacy[1][2], NEUTRAL_KIND);
        t.set_diplomacy(1, 2, 4).unwrap();
        assert_eq!(t.players[1].diplomacy[2], ENEMY_STANCE);
        assert_eq!(t.players[1].relations[2], 3);
        assert_eq!(t.spatial.unit_diplomacy[1][2], 4);
        let before = t.players.clone();
        t.set_diplomacy(1, 2, 2).unwrap();
        t.set_diplomacy(1, 9, 0).unwrap();
        t.set_diplomacy(1, -1, 0).unwrap();
        assert_eq!(t.players, before);
    }

    /// The world's teams: two players allied both ways with allied victory at 1 form team 2;
    /// one way, or without allied victory, they stay alone; locked teams stay.
    #[test]
    fn mutual_allies_with_allied_victory_form_a_team() {
        let mut t = world();
        t.set_diplomacy(1, 3, 0).unwrap();
        t.set_diplomacy(3, 1, 0).unwrap();
        assert_eq!(t.teams, [1; 8]);
        t.players[1].allied_victory = 1;
        t.players[3].allied_victory = 2;
        t.resolve_teams();
        assert_eq!(t.teams, [1; 8]);
        t.players[3].allied_victory = 1;
        t.resolve_teams();
        assert_eq!(t.teams[..3], [2, 1, 2]);
        t.settings.teams_locked = true;
        t.players[3].allied_victory = 0;
        t.resolve_teams();
        assert_eq!(t.teams[..3], [2, 1, 2]);
        t.settings.teams_locked = false;
        t.resolve_teams();
        assert_eq!(t.teams, [1; 8]);
    }

    /// Teams run out past 5: the players after the fourth team stay alone.
    #[test]
    fn the_teams_run_out_past_five() {
        let player = || Player {
            allied_victory: 1,
            ..Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0; 9], (0..9).map(|_| player()).collect());
        // pairs (1, 2), (3, 4), (5, 6), (7, 8), each allied both ways
        for (a, b) in [(1, 2), (3, 4), (5, 6), (7, 8)] {
            for (x, y) in [(a, b), (b, a)] {
                t.players[x].relations = vec![3; 9];
                t.players[x].relations[x] = 0;
                t.players[x].relations[y] = 0;
            }
        }
        t.resolve_teams();
        assert_eq!(t.teams, [2, 2, 3, 3, 4, 4, 5, 5]);
    }

    /// Attribute 204 truncated to 0 reveals nothing; 1 or more is not modelled.
    #[test]
    fn a_player_s_attribute_204_reveals_its_units() {
        let mut t = world();
        assert_eq!(t.reveal_units(), Ok(()));
        t.players[2].attributes[REVEAL_UNIT] = 0.5;
        assert_eq!(t.reveal_units(), Ok(()));
        t.players[2].attributes[REVEAL_UNIT] = -1.0;
        assert_eq!(t.reveal_units(), Err(REVEAL_UNITS));
        assert_eq!(t.set_diplomacy(1, 2, 0), Err(REVEAL_UNITS));
    }
}
