//! What a tick's update of one object reads and writes ([`Step`]) and its walks.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::combat::victims::KillLeft;
use crate::math::rng::{Stream, Well512};
use crate::movement::move_to::Walk;
use crate::movement::{Me, Planning};
use crate::objects::lifecycle::seconds;
use crate::objects::unit::Unit;
use crate::unit_ai::world_view::AiStart;
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
    /// The player the unit's own AI gave it to (slot 0x70, [`Unit::ai_change_owner`]) and
    /// whether it has the unit's master: the update ends with the takeover ([`crate::objects::unit::Outcome::Captured`]).
    pub(crate) taken_over: Option<(usize, bool)>,
    /// The unit is a building whose slot 0x70 ran in the update: the whole change is the
    /// caller's ([`crate::ownership::capture::Captured::building`]).
    pub(crate) building_handover: bool,
    /// The owner's id: the updating object's player, whose fields the update reads through
    /// the world (`w.players[player]`).
    pub(crate) player: usize,
    /// The actions the unit's slots 0x388 and 0x378 gave while its unit AI ran ([`AiStart`]),
    /// which the caller makes the unit's after the update, in order.
    pub(crate) starts: Vec<AiStart>,
}

/// What an object's step gathered in its `Step` for the world to apply after it
/// ([`crate::world::World::step_object`]): the `Step`'s fields of the same meaning.
pub(crate) struct ObjectOutputs {
    /// [`Step`]'s `starts`.
    pub(in crate::world) starts: Vec<AiStart>,
    /// [`Step`]'s `kills`.
    pub(in crate::world) kills: Vec<KillLeft>,
}

impl Step<'_> {
    /// What the step gathered ([`ObjectOutputs`]), taken out of the `Step`.
    pub(in crate::world) fn outputs(&mut self) -> ObjectOutputs {
        ObjectOutputs {
            starts: std::mem::take(&mut self.starts),
            kills: std::mem::take(&mut self.kills),
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

    /// [`Step::lend`] for a world routine on the updating object by its slot: `f` gets the world,
    /// the object's player and its index in the owner's live list.
    ///
    /// # Errors
    ///
    /// `f`'s and the lend's.
    pub(crate) fn lend_at<R>(
        &mut self,
        me: &mut Unit,
        f: impl FnOnce(&mut World, usize, usize) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        let (p, id) = (self.player, self.me);
        self.lend(me, |w| {
            let i = w.players[p].objects.position_of(id).ok_or_else(|| {
                GenieError::unreachable("a lent object left its list").with("object", id)
            })?;
            f(w, p, i)
        })
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
