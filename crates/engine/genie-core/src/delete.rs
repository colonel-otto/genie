//! A player's deletion of one of its objects: `CANCEL_BUILD` (0x6a, `TRIBE_Command::do_command`
//! case 0x6a), which the game's delete key sends.
//!
//! The command checks the object's owner and whether it may be deleted (`FUN_1416257c0`), then
//! runs its slot 0x8b8: a unit's (`FUN_14120f620`) takes its hit points (slot 0xd8), a building's
//! (`FUN_1411cd780` -> `FUN_1411ccbb0`) gives back what is left of a foundation's cost and takes
//! its hit points too. The object dies in its next update.

use crate::order::Place;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// The classes no delete takes (`DAT_142f925c0`, and `FUN_14120f620`'s own test): livestock,
/// class 42, artifacts and class 60.
const UNDELETABLE: [i16; 4] = [58, 42, 1, 60];

/// The first master type a delete takes (`FUN_1416257c0`: above `0x45`).
const DELETABLE_TYPE: u8 = 70;

/// The buildings' master type, whose slot 0x8b8 is the building's.
const BUILDING: u8 = 80;

/// A player's game status once it lost (`+0x89`).
const LOST: u8 = 2;

/// A town centre's master whose delete also tells the world's `+0x820` (`FUN_1414615d0`).
const TOWN_CENTRE_621: i16 = 621;

/// A capture flag (`+0x924`) under which no delete takes the object.
const BEING_CAPTURED: u8 = 2;

/// The class a unit's slot 0xd8 (`FUN_141211340`) spares, and the livestock it spares while it
/// holds some.
const SPARED_CLASS: i16 = 42;
const LIVESTOCK: i16 = 58;

impl Turns {
    /// `CANCEL_BUILD` for `player` on object `id`: the object must exist and be the player's,
    /// unless the setting `+0x1074` lets players order each other's ([`Turns::allied_control`])
    /// or its owner lost and the two are allies both ways (`FUN_14162ef10`). `FUN_1416257c0`
    /// then wants its deletion enabled (`+0x687`), a master of type 70 or more, a class other
    /// than [`UNDELETABLE`]'s and a capture flag (`+0x924`) other than 2; the object's slot 0x8b8
    /// follows ([`Turns::delete_unit`], [`Turns::delete_building`]). The bypass of the
    /// disable-verification cheat (`+0xb12`) is not held.
    ///
    /// # Errors
    ///
    /// What the deletes do not model.
    pub(crate) fn delete(&mut self, player: usize, id: i32) -> Result<(), Unsupported> {
        let Some((owner, unit)) = self.object(id) else {
            return Ok(());
        };
        let mutual =
            |a: usize, b: usize| self.spatial.allied_with(a, b) && self.spatial.allied_with(b, a);
        let lost_ally = self.players[owner].game_status == LOST && mutual(player, owner);
        if owner != player && !self.allied_control && !lost_ally {
            return Ok(());
        }
        let master = self.master_of(owner, unit.life.master_id)?;
        if unit.undeletable
            || master.kind < DELETABLE_TYPE
            || UNDELETABLE.contains(&master.class)
            || unit.capture_flag == BEING_CAPTURED
        {
            return Ok(());
        }
        if master.kind == BUILDING {
            self.delete_building(owner, id)
        } else {
            self.delete_unit(owner, id)
        }
    }

    /// A unit's slot 0x8b8 (`FUN_14120f620`): a unit of a class other than [`UNDELETABLE`]'s is
    /// marked deleted (`+0xa0c`) and its slot 0xd8 (`FUN_141211340`) spares class 42 and
    /// livestock holding some, else `FUN_14118f580`: a sleeping unit wakes to the end of its
    /// player's live list and its hit points go to 0. Not held: the deleted mark, the killer
    /// (`+0x690`, its owner) and the special-object list (`+0x30c` bit 0).
    fn delete_unit(&mut self, owner: usize, id: i32) -> Result<(), Unsupported> {
        let (_, unit) = self.object(id).expect("the caller's");
        let class = self.master_of(owner, unit.life.master_id)?.class;
        if UNDELETABLE.contains(&class) {
            return Ok(());
        }
        if class == SPARED_CLASS || (class == LIVESTOCK && 0.0 < unit.life.amount) {
            return Ok(());
        }
        let i = self.wake(id);
        self.players[owner].objects[i].life.hp = 0.0;
        Ok(())
    }

    /// A building's slot 0x8b8 (`FUN_1411cd780`, then `FUN_1411ccbb0`): it wakes; a foundation
    /// (below state 2) with at least 1 hit point gives its owner back the share of its paid cost
    /// (`+0x880`: each entry with a resource and the paid flag, slot 0xf8) its build time has
    /// still to go (`(build time - build points) / build time`, at most 1), all of it while no
    /// builder started it (`+0xc60`); then its hit points go to 0, and such a foundation is
    /// destroyed at once ([`Turns::destroy_foundation`]). Not modelled: master 621 (the world's
    /// `+0x820`, `FUN_1414615d0`). Not held: the deleted marks (`+0xa0c`, `+0xc`, `+0xb9c`), the
    /// semi-asleep list (`+0xc98`), the killer (`+0x690`) and a computer player's AI event (slot
    /// 0x380).
    fn delete_building(&mut self, owner: usize, id: i32) -> Result<(), Unsupported> {
        let (_, unit) = self.object(id).expect("the caller's");
        let master = self.master_of(owner, unit.life.master_id)?;
        if master.id == TOWN_CENTRE_621 {
            return Err(Unsupported(
                "a deleted town centre 621 (the world's +0x820)",
            ));
        }
        let cost = master.cost.clone();
        let (state, hp) = (unit.life.state, unit.life.hp);
        let site = unit.construction;
        let desolid = site.is_some_and(|c| c.desolid);
        let share = if state < 2 && 1.0 <= hp {
            let c = site.ok_or(Unsupported("a foundation without its construction"))?;
            let build_time = f32::from(c.build_time);
            (build_time - c.progress) / build_time
        } else {
            0.0
        };
        // at most 1 (a NaN keeps the 1)
        let mut share = if share <= 1.0 { share } else { 1.0 };
        if desolid && 0.0 < hp {
            share = 1.0;
        }
        if 0.0 < share {
            for (resource, amount, paid) in cost {
                if 0 <= resource && paid {
                    self.players[owner].add_attribute(resource, f32::from(amount) * share)?;
                }
            }
        }
        let i = self.wake(id);
        self.players[owner].objects[i].life.hp = 0.0;
        if state < 2 && desolid {
            self.destroy_foundation(owner, id)?;
        }
        Ok(())
    }

    /// A building's destroy (slot 0xd0, `FUN_1411cfb30`) of a foundation no builder started: its
    /// action list goes (it has none) and the base destroy (`FUN_14118f1e0`) takes its hit points
    /// and wakes it, then `set_state(7)` (`FUN_1411dbb70`: its annexes first, then the combat
    /// object's, `FUN_14121cc40`); its owner's watched list lets it go (`FUN_141091310`). Not
    /// modelled: a foundation with annexes. Not held: `+0xca4`.
    fn destroy_foundation(&mut self, owner: usize, id: i32) -> Result<(), Unsupported> {
        if !self.spatial.annexes_of(id).is_empty() {
            return Err(Unsupported("a deleted foundation with annexes"));
        }
        let i = self.wake(id);
        let life = &mut self.players[owner].objects[i].life;
        if life.state < 7 {
            life.hp = 0.0;
            life.state = 7;
            self.spatial.sight_state(id, 7);
        }
        self.unwatch(owner, id);
        Ok(())
    }

    /// `FUN_14119deb0` with 0: an object asleep wakes to the end of its player's live list, a
    /// live one flagged to sleep drops the flag; its index in the live list.
    fn wake(&mut self, id: i32) -> usize {
        match self.place(id).expect("the caller's") {
            Place::Live(p, i) => {
                self.players[p].objects[i].life.goto_sleep = false;
                i
            }
            Place::Sleeping(p, i) => {
                let mut woken = self.players[p].sleeping.swap_remove(i);
                woken.life.goto_sleep = false;
                self.players[p].objects.push(woken);
                self.players[p].objects.len() - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Construction;
    use crate::masters::{Master, Masters};
    use crate::spatial::Obstruction;
    use crate::turn::{Player, Unit};

    /// Gaia and two players: player 1's villager 10, house foundation 11 (build time 25, cost 25
    /// wood paid) and sheep 12, and player 2's villager 20.
    fn game(progress: f32, desolid: bool) -> Turns {
        let house = Unit {
            id: 11,
            construction: Some(Construction {
                progress,
                desolid,
                builders: 0,
                hit_points: 550,
                build_time: 25,
                record: Obstruction { kind: 2, class: 0 },
                snow: false,
                sprite_override: false,
                started: false,
            }),
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0; 3],
            vec![
                Player::default(),
                Player {
                    objects: vec![
                        Unit {
                            id: 10,
                            ..Unit::default()
                        },
                        house,
                        Unit {
                            id: 12,
                            ..Unit::default()
                        },
                    ],
                    attributes: vec![0.0; 10],
                    ..Player::default()
                },
                Player {
                    objects: vec![Unit {
                        id: 20,
                        ..Unit::default()
                    }],
                    ..Player::default()
                },
            ],
        );
        for (p, objects) in [
            (1, [(0, 83), (1, 70), (2, 594)].as_slice()),
            (2, &[(0, 83)]),
        ] {
            for &(i, master) in objects {
                let u = &mut t.players[p].objects[i];
                u.life.master_id = master;
                u.life.hp = 25.0;
            }
        }
        t.players[1].objects[1].life.state = 0;
        t.players[1].objects[2].life.amount = 100.0;
        let masters = Masters::new([
            Master {
                id: 83,
                class: 4,
                kind: 70,
                ..Master::default()
            },
            Master {
                id: 70,
                class: 3,
                kind: 80,
                cost: vec![(1, 25, true), (4, 1, false)],
                ..Master::default()
            },
            Master {
                id: 594,
                class: 58,
                kind: 70,
                ..Master::default()
            },
        ]);
        t.masters = vec![masters.clone(), masters.clone(), masters];
        t
    }

    #[test]
    fn a_delete_takes_a_unit_s_hit_points_but_not_another_player_s_or_livestock() {
        let mut t = game(0.0, true);
        t.delete(1, 10).unwrap();
        t.delete(1, 20).unwrap();
        t.delete(1, 12).unwrap();
        let hp = |t: &Turns, p: usize, i: usize| t.players[p].objects[i].life.hp;
        assert_eq!(
            (hp(&t, 1, 0), hp(&t, 2, 0), hp(&t, 1, 2)),
            (0.0, 25.0, 25.0)
        );
        // the setting +0x1074 lets a player delete another's
        t.allied_control = true;
        t.delete(1, 20).unwrap();
        assert_eq!(hp(&t, 2, 0), 0.0);
        // a unit whose deletion is disabled stays
        let mut t = game(0.0, true);
        t.players[1].objects[0].undeletable = true;
        t.delete(1, 10).unwrap();
        assert_eq!(hp(&t, 1, 0), 25.0);
    }

    #[test]
    fn a_deleted_foundation_gives_back_what_its_build_time_has_still_to_go() {
        // no builder started it: all of its paid cost, and it is destroyed at once (state 7)
        let mut t = game(0.0, true);
        t.delete(1, 11).unwrap();
        let house = &t.players[1].objects[1].life;
        assert_eq!(
            (t.players[1].resources[1], house.hp, house.state),
            (25.0, 0.0, 7)
        );
        // 10 of its 25 seconds of work done: 15/25 of the wood (in f32: 15.000001); it dies in
        // its own update
        let mut t = game(10.0, false);
        t.delete(1, 11).unwrap();
        let house = &t.players[1].objects[1].life;
        let share = (25.0_f32 - 10.0) / 25.0;
        assert_eq!(
            (t.players[1].resources[1], house.hp, house.state),
            (25.0 * share, 0.0, 0)
        );
    }
}
