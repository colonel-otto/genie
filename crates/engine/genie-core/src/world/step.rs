//! What a tick's update of one object reads and writes ([`Step`]) and its walks.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::combat::victims::KillLeft;
use crate::math::rng::{Stream, Well512};
use crate::movement::move_to::Walk;
use crate::movement::{Me, Planning};
use crate::objects::lifecycle::seconds;
use crate::objects::release::Releaser;
use crate::objects::unit::Unit;
use crate::unit_ai::AiMove;
use crate::unit_ai::unit::AiResume;
use crate::unit_ai::world_view::{AiStart, AiWork};
use crate::world::World;

pub(crate) struct Step<'a> {
    /// The world the update reaches (FD-045-D28): its objects by id, every player and the
    /// world's parts. The updating object is out of its slot (its stand-in, which no lookup by
    /// id finds), so a call on another object is the world's routine at once, and a call on
    /// the updating object itself goes to this `Step` for after the step.
    pub(crate) w: &'a mut World,
    /// The updating object's id.
    pub(crate) me: i32,
    /// The tick's dt in milliseconds (`world+0x164`; in seconds, [`seconds`]).
    ///
    /// [`seconds`]: crate::objects::lifecycle::seconds
    pub(crate) dt: u32,
    /// The ticks so far, this one included.
    pub(crate) tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    pub(crate) turn: Option<u32>,
    /// The unit AI's current target as its target block reads it ([`World::seen`]).
    pub(crate) ai_target: Result<Option<crate::unit_ai::Seen>, GenieError>,
    /// The objects about the target of a unit with an area effect (or about the unit, for its
    /// blast at death), which its blast may reach, read as the update starts; each hit writes
    /// them back where it ran ([`crate::combat::victims::write_back`]).
    pub(crate) blasted: Vec<Victim>,
    /// The updating object's own kill (a retaliation's), which its slot's stand-in keeps from
    /// the world until the step's end ([`crate::combat::victims::file_kill`]): the caller's
    /// after the update ([`World::kill_left`]); another object's kill reaches the world at the
    /// hit.
    pub(crate) kills: Vec<KillLeft>,
    /// The unit's update stopped before its unit AI for the caller's work order
    /// ([`Unit::ai_part`]); given back, the update runs only that part.
    pub(crate) ai_resume: Option<AiResume>,
    /// The player the unit's own AI gave it to (slot 0x70, [`Unit::ai_change_owner`]) and
    /// whether it has the unit's master: the update ends with the takeover ([`crate::objects::unit::Outcome::Captured`]).
    pub(crate) taken_over: Option<(usize, bool)>,
    /// The unit is a building whose slot 0x70 ran in the update: the whole change is the
    /// caller's ([`crate::ownership::capture::Captured::building`]).
    pub(crate) building_handover: bool,
    /// The releases (slot 0x180, [`World::release`]) and takes (slot 0x178, [`World::take`])
    /// of the updating object itself, which the view does not hold: the caller applies them
    /// after the hits' kills. The update calls every other one at once ([`crate::world::lets`]).
    pub(crate) releases: Vec<(i32, Releaser)>,
    pub(crate) takes: Vec<(i32, Releaser)>,
    /// A counted builder's leave ([`Unit::clear_own_actions`], `FUN_140aadbf0`) of the updating
    /// object itself, or of an annex of it: the building `FUN_140aaddb0` gives counts one
    /// builder less ([`World::leave_builders`]), the caller's before the releases.
    pub(crate) builder_leaves: Vec<i32>,
    /// The unit group a unit that entered a building left (`FUN_1411921d0`), with the
    /// building: the caller runs the group's removeUnit ([`World::remove_unit`]).
    pub(crate) left_group: Option<(i32, i32)>,
    /// The owner's id: the updating object's player, whose fields the update reads through
    /// the world (`w.players[player]`).
    pub(crate) player: usize,
    /// The work a builder found after its building (slot 0x7e8 from the Build's case 0xdf),
    /// which the caller gives after the update ([`World::order_work`]).
    pub(crate) work_order: Option<crate::orders::Work>,
    /// The Transport's unload in its state 6 (`FUN_140aa3ba0`), which the caller runs after the
    /// actions and before the unit AI part ([`World::transport_unload`]).
    pub(crate) transport_unload: bool,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`World::order_on_target`]), and the full update's tail that reads
    /// what the order gave ([`crate::unit_ai::UnitAi::after_order`]).
    pub(crate) order_on_target: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// The unit AI's `HuntObject`s, `GatherObject`s and works (slot 0x7e8) in the order it gave
    /// them, whose unit parts the caller runs after the update in that order ([`AiWork`]): a
    /// later one's stop tears down the action an earlier one made.
    pub(crate) works: Vec<AiWork>,
    /// The foundation of the unit's own build command (slot 0x380, a Gather's `set_state(6)` on
    /// a target not built yet), which the caller runs after the update ([`World::build_command`]).
    pub(crate) build_command: Option<i32>,
    /// A Gather's walk back to reseed ended (state 0xf2, whether it arrived), which the caller
    /// reseeds after the update ([`World::reseed_after_walk`]).
    pub(crate) reseed: Option<bool>,
    /// The unit AI's moves off foundations' footprints (slot 0x148), in order, which the caller
    /// runs after the update ([`World::move_aside`]).
    pub(crate) move_aside: Vec<AiMove>,
    /// The actions the unit's slots 0x388 and 0x378 gave while its unit AI ran ([`AiStart`]),
    /// which the caller makes the unit's after the update, in order.
    pub(crate) starts: Vec<AiStart>,
    /// The livestock a hunt sends to a drop site with its herd, which the caller sends after the
    /// update ([`World::lead_herd`]).
    pub(crate) leads: Option<(i32, i32)>,
}

/// What an object's step gathered in its `Step` for the world to apply after it
/// ([`crate::world::World::step_object`]): the `Step`'s fields of the same meaning; the left
/// group carries the unit's id.
pub(crate) struct ObjectOutputs {
    /// [`Step`]'s `order_on_target`.
    pub(in crate::world) ordered: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// [`Step`]'s `works`.
    pub(in crate::world) works: Vec<AiWork>,
    /// [`Step`]'s `leads`.
    pub(in crate::world) leads: Option<(i32, i32)>,
    /// [`Step`]'s `build_command`.
    pub(in crate::world) build_command: Option<i32>,
    /// [`Step`]'s `reseed`.
    pub(in crate::world) reseed: Option<bool>,
    /// [`Step`]'s `move_aside`.
    pub(in crate::world) move_aside: Vec<AiMove>,
    /// [`Step`]'s `starts`.
    pub(in crate::world) starts: Vec<AiStart>,
    /// [`Step`]'s `work_order`.
    pub(in crate::world) work_order: Option<crate::orders::Work>,
    /// [`Step`]'s `transport_unload`.
    pub(in crate::world) transport_unload: bool,
    /// [`Step`]'s `kills`.
    pub(in crate::world) kills: Vec<KillLeft>,
    /// [`Step`]'s `ai_resume`.
    pub(in crate::world) ai_resume: Option<AiResume>,
    /// [`Step`]'s `releases`.
    pub(in crate::world) releases: Vec<(i32, Releaser)>,
    /// [`Step`]'s `builder_leaves`.
    pub(in crate::world) builder_leaves: Vec<i32>,
    /// [`Step`]'s `left_group`.
    pub(in crate::world) left_group: Option<(i32, i32, i32)>,
    /// [`Step`]'s `takes`.
    pub(in crate::world) takes: Vec<(i32, Releaser)>,
}

impl Step<'_> {
    /// What the step gathered ([`ObjectOutputs`]), taken out of the `Step`; `id` is the
    /// unit's.
    pub(in crate::world) fn outputs(&mut self, id: i32) -> ObjectOutputs {
        ObjectOutputs {
            ordered: self.order_on_target.take(),
            works: std::mem::take(&mut self.works),
            leads: self.leads.take(),
            build_command: self.build_command.take(),
            reseed: self.reseed.take(),
            move_aside: std::mem::take(&mut self.move_aside),
            starts: std::mem::take(&mut self.starts),
            work_order: self.work_order.take(),
            transport_unload: std::mem::take(&mut self.transport_unload),
            kills: std::mem::take(&mut self.kills),
            ai_resume: self.ai_resume.take(),
            releases: std::mem::take(&mut self.releases),
            builder_leaves: std::mem::take(&mut self.builder_leaves),
            left_group: self.left_group.take().map(|(g, b)| (g, id, b)),
            takes: std::mem::take(&mut self.takes),
        }
    }
}

/// What a walk takes of the update beside the world: the dt in seconds, the world's time, and
/// the owner's computer flag and construction rate.
#[derive(Clone, Copy)]
pub(crate) struct Pace {
    pub(crate) dt: f32,
    pub(crate) now: u32,
    pub(crate) computer: bool,
    pub(crate) construction_rate: f32,
}

/// The world's parts beside a walk ([`World::walk_beside`]), which an update reads and changes
/// while its unit walks.
pub(crate) struct Beside<'b> {
    /// The sprites' table.
    pub(crate) graphics: &'b std::collections::BTreeMap<i16, crate::objects::sprite::Graphic>,
    /// The Sprite stream, which a sprite the walk shows draws from.
    pub(crate) sprite_rng: &'b mut Well512,
}

impl World {
    /// A walk at `pace` ([`Step::walk`]) and the world's parts beside it, each its own
    /// borrow.
    pub(crate) fn walk_beside<'b>(
        &'b mut self,
        pace: Pace,
        body: &'b mut crate::objects::layers::Body,
        me: Me,
    ) -> (Walk<'b>, Beside<'b>) {
        let Self {
            spatial,
            pathing,
            groups,
            streams,
            graphics,
            ..
        } = self;
        let (object, sprite_rng) = streams.two(Stream::Object, Stream::Sprite);
        let walk = Walk {
            body,
            me,
            dt: pace.dt,
            world: spatial,
            plan: Some(Planning {
                pathing,
                now: pace.now,
                computer: pace.computer,
                object,
                construction_rate: pace.construction_rate,
                groups,
            }),
        };
        let beside = Beside {
            graphics,
            sprite_rng,
        };
        (walk, beside)
    }
}

impl Step<'_> {
    /// Runs `f` on the world with the updating object `me` lent back to its slot (FD-045-Q03:
    /// the game's world holds the object while its update calls a routine that scans the
    /// objects, a technology's effects or a search among them), then takes it out again from
    /// where `f` left it; `me` holds what `f` changed on it.
    ///
    /// # Errors
    ///
    /// `f`'s, or the object's stand-in or the object itself gone from the owner's live list.
    pub(crate) fn lend<R>(
        &mut self,
        me: &mut Unit,
        f: impl FnOnce(&mut World) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        let p = self.player;
        let lent = std::mem::replace(me, Unit::taken_out());
        let answer = self.w.put_back_own(p, lent).and_then(|_| f(self.w));
        let own = &mut self.w.players[p].objects;
        let i = own.position_of(self.me).ok_or_else(|| {
            GenieError::unreachable("a lent object left its list").with("object", self.me)
        })?;
        *me = own.take_out(i);
        answer
    }

    /// The owner's slot 0xf8 with 1 and 1 on a unit's load (`FUN_141616820`,
    /// [`give_load`]): the owner gains the resource and its gathered total at once.
    ///
    /// [`give_load`]: crate::gathering::gather::unit::give_load
    ///
    /// # Errors
    ///
    /// [`give_load`]'s, or an attribute the owner does not have.
    pub(crate) fn give_load(&mut self, kind: i16, amount: f32) -> Result<(), GenieError> {
        let mut adds = Vec::new();
        crate::gathering::gather::unit::give_load(
            (
                &self.w.spatial.relations,
                self.w.players[self.player].sharing,
            ),
            self.player,
            &mut adds,
            kind,
            amount,
        )?;
        self.w.players[self.player].add_attributes(adds)
    }

    /// What a walk takes of the update beside the world ([`World::walk_beside`]).
    pub(crate) fn pace(&self) -> Pace {
        Pace {
            dt: seconds(self.dt),
            now: self.w.world_time,
            computer: self.w.players[self.player].computer,
            construction_rate: self.w.players[self.player].construction_rate,
        }
    }
}

impl Step<'_> {
    /// A walk and the Sprite stream, for an attacker's update.
    pub(crate) fn walk_and_sprites<'b>(
        &'b mut self,
        body: &'b mut crate::objects::layers::Body,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, sprite) = self.w.streams.two(Stream::Object, Stream::Sprite);
        (
            Walk {
                body,
                me,
                dt: seconds(self.dt),
                world: &mut self.w.spatial,
                plan: Some(Planning {
                    pathing: &mut self.w.pathing,
                    now: self.w.world_time,
                    computer: self.w.players[self.player].computer,
                    object,
                    construction_rate: self.w.players[self.player].construction_rate,
                    groups: &self.w.groups,
                }),
            },
            sprite,
        )
    }

    /// A walk with the Sprite and the Action streams (a farmer's stops draw from the second).
    pub(crate) fn walk_sprites_action<'b>(
        &'b mut self,
        body: &'b mut crate::objects::layers::Body,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512, &'b mut Well512) {
        let [object, sprite, action] =
            self.w
                .streams
                .three(Stream::Object, Stream::Sprite, Stream::Action);
        (
            Walk {
                body,
                me,
                dt: seconds(self.dt),
                world: &mut self.w.spatial,
                plan: Some(Planning {
                    pathing: &mut self.w.pathing,
                    now: self.w.world_time,
                    computer: self.w.players[self.player].computer,
                    object,
                    construction_rate: self.w.players[self.player].construction_rate,
                    groups: &self.w.groups,
                }),
            },
            sprite,
            action,
        )
    }

    /// What an object's walking action needs from the tick.
    pub(crate) fn walk<'b>(
        &'b mut self,
        body: &'b mut crate::objects::layers::Body,
        me: Me,
    ) -> Walk<'b> {
        Walk {
            body,
            me,
            dt: seconds(self.dt),
            world: &mut self.w.spatial,
            plan: Some(Planning {
                pathing: &mut self.w.pathing,
                now: self.w.world_time,
                computer: self.w.players[self.player].computer,
                object: self.w.streams.get(Stream::Object),
                construction_rate: self.w.players[self.player].construction_rate,
                groups: &self.w.groups,
            }),
        }
    }
}
