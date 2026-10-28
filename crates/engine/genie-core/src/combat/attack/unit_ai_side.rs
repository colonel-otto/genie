//! The attack the unit AI commands while it runs ([`crate::unit_ai::UnitSide::attack`] through
//! its view, [`UnitView`]): the new Attack, a forced attack again on the unit's own target, the
//! target as a hit reads it, the start's readiness, the target's sight and a villager's swap.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::combat::victims::{victim_in, victim_of};
use crate::map::spatial::Placed;
use crate::movement::Me;
use crate::movement::move_to::Walk;
use crate::objects::release::Releaser;
use crate::unit_ai::UnitAi;
use crate::unit_ai::task_finder::world::SideWorld;
use crate::unit_ai::world_view::{NOT_HELD, UnitView};

/// The villagers a villager's attack command switches to by its task swap group (its player's
/// master table's entries at `+0x298` and `+0x928`).
const VILLAGER_MALE: i16 = 83;
const VILLAGER_FEMALE: i16 = 293;

impl<'u> UnitView<'u> {
    /// The attack command (slot 0x338, `FUN_14120cea0`; classes 51 and 54 are not modelled) ->
    /// `FUN_141081a80`: a target the world does not hold fails; `+0x648` takes the world's time
    /// (not held); the unit's own Attack on the target answers true, forced after its slot 0x48
    /// on it ([`UnitView::attack_again`]); a villager's master switch (slot 0x90) is not modelled.
    /// Then the new
    /// Attack ([`crate::combat::attack::Attack::command`]): the constructor takes its target (slot 0x178,
    /// the caller's) and clears the unit's `+0x900` (its target is new), the action list clears
    /// (the old actions' teardowns), and the install's start (`0x140a7f8c0`) of a unit inside
    /// another fails it at once (`set_state(13)`, [`crate::combat::attack::Attack::fail_at_start`]); else
    /// it puts the unit on its standing sprite (slot 0x68 with `+0x80`) and runs the range test
    /// ([`crate::combat::attack::Attack::begin`]): readyToAttack's failure
    /// before a `set_state(4)` leaves the animation at speed 1; a `MoveTo` that starts arrived
    /// shows the standing sprite; a unit that cannot move in reach goes ready (`set_state(6)`,
    /// [`crate::combat::attack::Attack::enter_ready`]: readyToAttack again, then the swing, `set_state(7)`,
    /// when it faces its target and may attack), whose notifies join the start's. A notify posted
    /// while the AI runs (its buffer, `FUN_140c55270`) is not modelled.
    pub(crate) fn start_attack_now(
        &mut self,
        target: i32,
        force: bool,
        ai: &UnitAi,
    ) -> Result<crate::unit_ai::Attacked, GenieError> {
        let unit = &mut *self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(crate::world::ATTACKER_MASTER)?;
        if matches!(master.class, 51 | 54) {
            return Err(GenieError::unsupported(
                "the attack command of a unit of class 51 or 54",
            ));
        }
        let no = crate::unit_ai::Attacked::default;
        let Some(t) = self.spatial.get_known(target)?.copied() else {
            return Ok(no());
        };
        if unit.actions.attack().is_some_and(|a| a.target == target) {
            if force {
                self.attack_again(target, &t, master, ai)?;
            }
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        // a villager switches to its player's villager of its task swap group (`+0x7b8` 1: the
        // master table's `+0x298`, 83; else `+0x928`, 293) when the player has it (slot 0x90 with
        // 0: the action list clears, then the master's unit part; its owner's part after the
        // update)
        let master = match self.villager_swap(master) {
            Some(next) => {
                self.switch_unit_master(master, next)?;
                next
            }
            None => master,
        };
        let me_at = *self
            .spatial
            .get(self.unit.id)
            .ok_or(crate::world::ATTACKER_UNPLACED)?;
        let t_master = self
            .every
            .get(t.player)
            .and_then(|m| m.get(t.master))
            .ok_or(GenieError::unsupported(
                "an attack's target of a master the slice does not have",
            ))?;
        let visible = self.target_visible(&t, t_master)?;
        let unit = &mut *self.unit;
        // the attack command's Attack (`FUN_141081a80`) takes no MoveTo sprite: the unit's
        let look = unit.abilities.look.as_ref();
        let arms = crate::combat::attack::arms_looking(master, look, self.graphics, [-1; 4], -1);
        let standing = arms.idle_sprite.ok_or(crate::world::NO_STANDING_SPRITE)?;
        if unit.sprite.is_none() {
            return Err(GenieError::unsupported(
                "an attack command of a unit whose sprite the slice does not hold",
            ));
        }
        // the action list's clearing (FUN_140a98620): an Attack on another target tears down (its
        // MoveTo's stops the unit) and lets its targets go (slot 0x180), as do the others
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        let speed = unit
            .body
            .mover()
            .ok_or(crate::world::ATTACKER_UNMOVING)?
            .stats
            .speed;
        let e = crate::combat::attack::engagement(
            Some(ai),
            speed,
            master,
            &me_at,
            &t,
            self.world_time,
        )?;
        // the constructor clears +0x900 for a target other than its own (none yet)
        unit.attack_count = 0;
        if unit.life.inside.is_some() {
            let mut attack = crate::combat::attack::Attack::own(target, &e, arms);
            attack.fail_at_start(&mut unit.sprite, &mut unit.body, self.sprite)?;
            unit.actions.set_attack(Some(attack));
            self.takes.push((target, by));
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        crate::combat::attack::set_sprite(&mut unit.sprite, standing, self.sprite);
        let mover = unit.body.mover_mut().expect("its speed read above");
        let crate::combat::attack::Commanded {
            mut attack,
            arrived,
            mut posted,
            ready,
            failed,
            failed_first,
        } = crate::combat::attack::Attack::command(
            (target, self.spatial),
            &e,
            arms,
            (visible, t.player),
            mover,
        )?;
        if failed_first {
            unit.body.set_anim_speed(1.0);
        }
        if failed {
            attack.fail_at_start(&mut unit.sprite, &mut unit.body, self.sprite)?;
        }
        if ready {
            posted.extend(self.ready_at_start(&mut attack, master, ai)?);
        }
        let unit = &mut *self.unit;
        let mover = unit.body.mover_mut().expect("its speed read above");
        if arrived {
            // the MoveTo's start in state 1 (`0x140aa29e0`, not quiet): the master's standing
            // sprite with its speed, `+0x7b8` cleared; the next update's state 4 ends the MoveTo
            mover.shows_standing();
            unit.shows_standing_sprite(self.masters, self.graphics, self.sprite)?;
        }
        unit.actions.set_attack(Some(attack));
        self.takes.push((target, by));
        Ok(crate::unit_ai::Attacked {
            attacks: true,
            posted,
        })
    }

    /// The forced attack command on the target of the unit's own Attack (`FUN_141081a80`): the
    /// top Attack's slot 0x48 (`FUN_140a841d0`, [`crate::combat::attack::Attack::work`]) on the target
    /// and its place, with the main building of an annex for a unit of an area effect, the
    /// unit's top speed and canMoveToAttack ([`crate::unit_ai::Seeker::can_move`]); what it takes
    /// and lets go is the caller's ([`UnitView::takes`], [`UnitView::releases`]), a new target
    /// clearing the unit's attack count (`+0x900`). Its answer is not read.
    fn attack_again(
        &mut self,
        target: i32,
        t: &Placed,
        master: &crate::objects::masters::Master,
        ai: &UnitAi,
    ) -> Result<(), GenieError> {
        let mark = |id: i32, p: &Placed| -> Result<crate::combat::attack::Mark, GenieError> {
            let (_, u) = self
                .foreign
                .find(id)
                .or_else(|| self.others.iter().find(|u| u.id == id).map(|u| (0, u)))
                .ok_or(NOT_HELD)?;
            let m = self
                .every
                .get(p.player)
                .and_then(|ms| ms.get(p.master))
                .ok_or(NOT_HELD)?;
            Ok(crate::combat::attack::Mark {
                id,
                kind: m.kind,
                class: m.class,
                state: u.life.state,
                position: p.position,
                radius: p.radius,
            })
        };
        let target_mark = mark(target, t)?;
        let main = match self
            .foreign
            .find(target)
            .map(|(_, u)| u.annex_of)
            .or_else(|| {
                self.others
                    .iter()
                    .find(|u| u.id == target)
                    .map(|u| u.annex_of)
            })
            .flatten()
        {
            Some(m) => match self.spatial.get(m).copied() {
                Some(p) => Some(mark(m, &p)?),
                None => None,
            },
            None => None,
        };
        let unit = &mut *self.unit;
        let wielder = crate::combat::attack::Wielder {
            area_effect: master.area_effect,
            speed: unit.body.mover().map_or(0.0, |m| m.stats.speed),
            position: unit.position,
            radius: master.radius,
        };
        let can_move = ai.current_order != -1 || ai.stance <= 1;
        let attack = unit.actions.attack_mut().expect("the caller's own Attack");
        let w = attack.work(Some(target_mark), main, &wielder, |_| can_move);
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        if w.clears_count {
            unit.attack_count = 0;
        }
        if let Some(old) = w.lets_go {
            self.releases.push((old, by));
        }
        if let Some(new) = w.takes {
            self.takes.push((new, by));
        }
        Ok(())
    }

    /// What a hit of the unit reads of the object `id` ([`victim_in`]): one of the owner's live
    /// objects or another player's; None for an object the view does not hold (the owner's
    /// sleeping objects, or the old owner's of a unit taken over in its update, whose attributes
    /// it does not hold).
    fn victim_of_target(&self, id: i32) -> Option<Victim> {
        let (q, t, attributes) = if let Some(t) = self.others.iter().find(|u| u.id == id) {
            (self.player, t, self.attributes)
        } else {
            let (q, t) = self.foreign.find(id)?;
            (q, t, &self.foreign.player(q)?.attributes[..])
        };
        let master = self.every.get(q)?.get(t.life.master_id)?;
        let dip = SideWorld {
            player: self.player,
            relations: self.relations,
            foreign: self.foreign,
            drop_sites: self.drop_sites,
        };
        Some(victim_in(
            t,
            (q, master, attributes),
            (self.player, &dip),
            self.spatial,
            self.mode_680,
        ))
    }

    /// The start's `set_state(6)` of the unit's new Attack ([`crate::combat::attack::Attack::enter_ready`])
    /// with what its readyToAttack and its swing read: the target as a hit reads it
    /// ([`UnitView::victim_of_target`]), the unit as a combat object, its unit AI's stance, leash
    /// and queue. Returns the notifies it posted. Its swing lands no hit and fires nothing before
    /// its update, nor does readyToAttack let the target go or send the unit home there.
    fn ready_at_start(
        &mut self,
        attack: &mut crate::combat::attack::Attack,
        master: &crate::objects::masters::Master,
        ai: &UnitAi,
    ) -> Result<Vec<(i32, [i32; 3])>, GenieError> {
        let mut victim = self
            .victim_of_target(attack.target)
            .ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
        let unit = &mut *self.unit;
        let own = victim_of(
            unit,
            master,
            self.attributes,
            self.player,
            self.spatial.terrain.as_ref(),
            self.mode_680,
        );
        let me = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: crate::combat::attack::READY,
        };
        let walk = Walk {
            mover: unit.body.mover_mut(),
            me,
            dt: 0.0,
            world: self.spatial,
            plan: None,
        };
        let mut blasted = Vec::new();
        let mut strike = crate::combat::attack::Strike {
            sprite: &mut unit.sprite,
            standing: self.graphics.get(&master.standing_sprite).copied(),
            volleys: self
                .masters
                .is_some_and(|m| crate::combat::attack::volleys(master, m)),
            attack_timer: &mut unit.attack_timer,
            reload_worldtime: &mut unit.reload_worldtime,
            reload: unit.reload_stinger + (master.reload - unit.reload_powerup),
            hit_chance: master.hit_chance,
            victim: &mut victim,
            blasted: &mut blasted,
            sprite_rng: self.sprite,
            own,
            executed: None,
            fire: None,
            can_move: ai.current_order != -1 || ai.stance <= 1,
            stance: ai.stance,
            state_position: ai.state_position,
            posted: Vec::new(),
            queue_empty: ai.orders.is_empty(),
            world_time: self.world_time,
            ai_order: Some(ai.current_order),
            ai_type: Some(ai.ai_type),
            computer: self.computer,
            armed: crate::combat::volley::armed(master, unit.firepower),
            volley_under_way: unit.next_volley != 0,
            let_go: None,
            forgets: false,
            sends_home: false,
            swing_sprites: crate::combat::attack::swing_sprites(
                unit.abilities.look.as_ref(),
                master,
                self.graphics,
            ),
        };
        attack.enter_ready(walk, &mut strike)?;
        // set_state 6 and 7 hit nothing, fire nothing and let nothing go: the update does
        debug_assert!(strike.executed.is_none() && strike.fire.is_none());
        debug_assert!(strike.let_go.is_none() && !strike.forgets && !strike.sends_home);
        Ok(std::mem::take(&mut strike.posted))
    }

    /// The attack command's sight of its target `t` (of master `t_master`): its tile seen by the
    /// unit's player, or explored when the master shows in the fog (its fog flag).
    ///
    /// # Errors
    ///
    /// A tile the map's sight does not hold.
    fn target_visible(
        &self,
        t: &Placed,
        t_master: &crate::objects::masters::Master,
    ) -> Result<bool, GenieError> {
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        Ok(self.spatial.sees_tile(self.player, tx, ty)
            || (t_master.fog_visibility != 0 && self.spatial.explored(self.player, tx, ty)?))
    }

    /// The attack command's villager swap: a villager (class 4) switches to its player's villager
    /// of its task swap group (`+0x7b8` 1: the master table's `+0x298`, 83; else `+0x928`, 293)
    /// when the player has it.
    fn villager_swap(
        &self,
        master: &crate::objects::masters::Master,
    ) -> Option<&'u crate::objects::masters::Master> {
        if master.class != 4 {
            return None;
        }
        let swap = if master.task_by_group == 1 {
            VILLAGER_MALE
        } else {
            VILLAGER_FEMALE
        };
        self.masters.and_then(|m| m.get(swap))
    }
}
