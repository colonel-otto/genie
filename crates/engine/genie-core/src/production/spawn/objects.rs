//! A spot beside a building, the remains a death leaves, a plain object, the flares and a
//! building a death makes.

use crate::GenieError;
use crate::map::Point;
use crate::map::Position;
use crate::map::spatial::{Obstruction, Placed};
use crate::math::rng::Stream;
use crate::movement::Mover;
use crate::objects::ObjectId;
use crate::objects::lifecycle::{Class, Lifecycle};
use crate::objects::masters::MasterId;
use crate::objects::masters::class;
use crate::objects::masters::master_type::ANIMATED;
use crate::objects::masters::master_type::BUILDING;
use crate::objects::masters::master_type::COMBAT;
use crate::objects::masters::master_type::MOVING;
use crate::objects::masters::master_type::STATIC;
use crate::objects::sprite::ActiveSprite;
use crate::objects::unit::Unit;
use crate::players::PlayerId;
use crate::production::spawn::{
    Facing, NOT_MOVING, Placing, Site, WorldMap, corner_tiles, placement, spawn_point,
};
use crate::vision::visibility::{Seer, capture_kind};
use crate::world::World;

impl World {
    /// `FUN_1411d12d0` for a unit of `made` beside the building at `i` of player `p`, `toward`
    /// the point the search faces (its gather point's, or the command's): [`spawn_point`] with
    /// the walls at the building's corners (class 27 on both tiles beside each) and the master's
    /// placement test at each candidate ([`placement`]).
    pub(crate) fn spot_beside(
        &self,
        p: PlayerId,
        i: usize,
        made: &crate::objects::masters::Master,
        toward: Option<[f32; 2]>,
    ) -> Result<[f32; 2], GenieError> {
        let building = &self.players[p.index()].objects[i];
        let site_master = building.master(&self.masters, p)?;
        let position = building.position();
        let mut walls = [false; 4];
        for (wall, tiles) in
            walls
                .iter_mut()
                .zip(corner_tiles(position, site_master.radius, made.radius))
        {
            for &(x, y) in &tiles {
                let (fx, fy) = (x as f32, y as f32);
                self.spatial.check_known([fx, fy, fx + 1.0, fy + 1.0])?;
            }
            *wall = tiles.iter().all(|&(x, y)| {
                self.spatial
                    .object_on_tile(x, y, |o| o.class == class::WALL)
            });
        }
        let site = Site {
            position,
            radius: site_master.radius,
            gather: toward,
            walls,
        };
        let moving = made.moving.ok_or(NOT_MOVING)?;
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| t.restrictions.get(moving.restriction))
            .ok_or(GenieError::unsupported(
                "a unit's terrain restriction the data file does not have",
            ))?;
        let placing = Placing {
            player: p,
            radius: made.radius,
            tall: made.tall,
            class: made.class,
            restriction,
            hill_mode: made.hill_mode,
            placement: made.placement,
        };
        let map = WorldMap { turns: self };
        let spatial = &self.spatial;
        spawn_point(&site, made.radius, &mut |[x, y]| {
            let [rx, ry] = made.radius;
            spatial.check_known([x - rx, y - ry, x + rx, y + ry])?;
            placement(&placing, &map, [x, y], false)
        })
    }

    /// `FUN_1411a4e90`: the remains a dying sprite's end leaves for player `p`: the master's slot
    /// 0x40 makes an object of `master` at the dead object's place `at` ([`World::plain_object`]
    /// facing the dead object's facet, `FUN_14119d490` and `FUN_14114d780`). It joins the owner's
    /// list and updates later in the same tick. Remains of type 80 ([`World::new_building`]),
    /// of type 70 (a combat unit, [`World::combat_object`]: a placeholder villager's death spawn,
    /// masters 1875 and 1876, is a villager) and the plain objects' types are modelled; not held:
    /// the remains' sprite.
    pub(crate) fn remains(
        &mut self,
        p: PlayerId,
        master: MasterId,
        at: Position,
        facet: Option<u8>,
    ) -> Result<(), GenieError> {
        let made = self.master(p, master)?.clone();
        if made.kind == BUILDING {
            return self.new_building(p, &made, at);
        }
        if made.kind == COMBAT {
            let new = self.combat_object(p, master, &made, at)?;
            // a moving dead object's override (FUN_14119d490, FUN_14114d780): the new unit takes
            // its facet's angle
            if let Some(facet) = facet {
                let standing = self.graphics.get(&made.standing_sprite).copied();
                let own = standing.map_or(
                    crate::objects::facing::SpriteFacets {
                        count: 0,
                        flag_1: false,
                    },
                    |g| crate::objects::facing::SpriteFacets {
                        count: g.facets,
                        flag_1: g.flags & 2 != 0,
                    },
                );
                if let Some(m) = self.players[p.index()].objects[new].body.mover_mut() {
                    m.angle = crate::objects::facing::angle_of_facet(facet, own);
                }
            }
            return Ok(());
        }
        let _ = self.plain_object(p, master, &made, at, Facing::Dead(facet))?;
        Ok(())
    }

    /// The plain object's sight: its seer joins its owner's map and its sight comes on.
    fn seer_comes(
        &mut self,
        id: i32,
        (p, master): (PlayerId, MasterId),
        made: &crate::objects::masters::Master,
        at: Position,
    ) -> Result<(), GenieError> {
        if let Some(v) = self.spatial.sight.as_mut() {
            v.add_seer(
                id,
                Seer {
                    player: p,
                    master,
                    los: made.line_of_sight,
                    state: 2,
                    explored: false,
                    building: None,
                    umv: 0,
                    capture: capture_kind(p, made.class, master),
                    kind: made.kind,
                },
            );
            let _ = v.sight_on(id, at, made.radius, false, false)?;
        }
        Ok(())
    }

    /// The master's slot 0x40 for a static object (type 10) or an animated or moving one (types
    /// 20 to 39) of `made` (id `master`) for player `p` at `at`, with the setups' draws (static
    /// `FUN_1411a45a0`: its standing sprite, [`ActiveSprite::start`] on the Sprite stream, then
    /// its facet on the Object stream for a sprite with a random first frame and several facets;
    /// moving `FUN_141141640` after the animated setup: its angle on the Object stream, sequence
    /// flag 2, which `facing` may replace), its id (`FUN_14152fd70`), its place, record and mesh
    /// shape (slot 0x58), its sight and its storage (flags 1, 2 and 32 to its owner,
    /// `FUN_141193860`; the last entry of flag 0 held, `FUN_141193710`). A static object takes
    /// its death on the sprite after it with a random facet (`FUN_141126300` ->
    /// `FUN_141169580`); a moving one faces as `facing` says. An object with hit points is filed
    /// asleep by the static setup (`+0x641`, the player's slot 0x2d0), and the animated setup
    /// wakes it at once (`FUN_14119d490` with 0: off the end of the sleeping list, slot 0x2d8,
    /// onto the live list's, slot 0x2d0, the tracker filing it last): only a static one would
    /// sleep (not modelled). It joins the owner's list last; its index there. A new object
    /// facing its drawn angle holds its sprite; remains do not. Not held: its facet (`+0xe`).
    ///
    /// # Errors
    ///
    /// Another type, a static object with hit points, a recyclable master, the moving remains
    /// of an object without a facet the slice holds, and what the sight does not model.
    pub(crate) fn plain_object(
        &mut self,
        p: PlayerId,
        master: MasterId,
        made: &crate::objects::masters::Master,
        at: Position,
        facing: Facing,
    ) -> Result<usize, GenieError> {
        if made.kind != STATIC && !(20..40).contains(&made.kind) {
            return Err(GenieError::unsupported(
                "remains of a type other than 10 and 20 to 39",
            ));
        }
        if made.kind == STATIC && 0 < made.hit_points {
            return Err(GenieError::unsupported(
                "remains with hit points (the sleeping list)",
            ));
        }
        if made.recyclable {
            return Err(GenieError::unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        // its place (slot 0x58, the static new_position): the ground's height by its master's
        // fly mode (slot 0x60, FUN_141193a10)
        let at = self.on_ground(p, master, at);
        let id = self.take_object_id(p, master);
        let standing = (0 <= made.standing_sprite)
            .then(|| self.graphics.get(&made.standing_sprite).copied())
            .flatten();
        let mut sprite = None;
        if let Some(g) = standing {
            sprite = Some(ActiveSprite::start(g, self.streams.get(Stream::Sprite)));
            if g.flags & 4 != 0 && 1 < g.facets {
                let _ = self.streams.get(Stream::Object).rand15();
            }
        }
        let facets = standing.map(|g| crate::objects::facing::SpriteFacets {
            count: g.facets,
            flag_1: g.flags & 2 != 0,
        });
        let mut angle = 0.0;
        let moving = if MOVING <= made.kind {
            // the moving setup's angle draw (FUN_141141640, "setup" line 0x2c3)
            let drawn = if crate::objects::facing::draws_angle(facets) {
                let r = self.streams.get(Stream::Object).rand15();
                facets.map(|f| crate::objects::facing::drawn_angle(r, f))
            } else {
                None
            };
            angle = match facing {
                // no draw leaves the constructor's 0
                Facing::Drawn => drawn.unwrap_or(0.0),
                // the dead object's facet replaces it: that facet's angle (FUN_14119d490,
                // FUN_14114d780, with the remains' own sprite's facets)
                Facing::Dead(facet) => {
                    let facet = facet.ok_or(GenieError::unsupported(
                        "moving remains of an object without a facet the slice holds",
                    ))?;
                    let own = facets.unwrap_or(crate::objects::facing::SpriteFacets {
                        count: 0,
                        flag_1: false,
                    });
                    crate::objects::facing::angle_of_facet(facet, own)
                }
            };
            Some(made.moving.ok_or(GenieError::unsupported(
                "moving remains without a moving master",
            ))?)
        } else {
            None
        };
        let solid = 0.0 < made.radius[0] && 0.0 < made.radius[1] && made.tall;
        let placed = Placed {
            position: at,
            radius: made.radius,
            class: made.class,
            player: p,
            master,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: moving.as_ref().map_or(0.0, |m| m.speed),
            unknown: None,
            obstruction: (made.obstruction_type != 0 && solid).then_some(Obstruction {
                kind: made.obstruction_type,
                class: made.obstruction_class,
            }),
        };
        self.spatial.insert(id, placed);
        self.pathing.moved(id.get(), &placed, false);
        self.pathing.tiles_changed(self.spatial.take_changes());
        self.seer_comes(id.get(), (p, master), made, at)?;
        if let Some(e) = self.take_sight_unsupported() {
            return Err(e);
        }
        let owner = &mut self.players[p.index()];
        let mut held = (-1, 0.0);
        for &(kind, amount, flag) in &made.storage {
            if kind >= 0 && matches!(flag, 1 | 2 | 32) {
                owner.add_attribute(kind, amount)?;
            }
            if kind != -1 && flag == 0 {
                held = (kind, amount);
            }
        }
        // a static object (type 10) dies on its sprite after its death (the dying one, else its
        // standing one), which may start on a random facet
        let death_facets = if made.kind == STATIC {
            let after = if 0 <= made.death_sprite {
                made.death_sprite
            } else {
                made.standing_sprite
            };
            self.graphics
                .get(&after)
                .filter(|g| g.flags & 4 != 0 && 1 < g.facets)
                .map(|g| g.facets)
        } else {
            None
        };
        owner.objects.push(Unit {
            id,
            life: Lifecycle {
                class: if made.kind == STATIC {
                    Class::Static
                } else {
                    Class::Animated
                },
                state: 2,
                hp: f32::from(made.hit_points),
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: (0 <= made.death_sprite).then_some(made.death_sprite),
                death_spawn: made.death_spawns[0].is_some(),
                remains: made.death_spawns[1],
                death_facets,
                inside: None,
                goto_sleep: false,
                master_id: master,
                master: made.life,
            },
            // an animated one's constructor (FUN_141070cf0): animation speed (+0x7b8) 0
            body: crate::objects::layers::Body::new(
                at,
                (ANIMATED <= made.kind).then(crate::objects::layers::Animated::default),
                moving.map(|stats| Mover::new_object(stats, at, self.world_time, angle, facets)),
            ),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            sprite: sprite.filter(|_| facing == Facing::Drawn),
            ..Unit::default()
        });
        let idx = owner.objects.len() - 1;
        // the static init's add to its player (slot 0x2d0) files it in the tracker
        self.track_add(p, id.get(), master).map(|()| idx)
    }

    /// FLARE's objects (`0x140e7bfd0`): with `players` at most the world's players, each player
    /// from 1 below `players` makes a flare (master 274, its slot 0x1c0 `FUN_14162ce00` with the
    /// footprint left alone: the master's slot 0x40 as [`World::remains`] makes it, then
    /// [`World::made_new_object`]) at
    /// `target`'s place, or at `point` on the ground when `target` is −1 or gone; its workers
    /// (`+0x644`) are 0. Not held: the sender's flare's workers 1 when the local player is among
    /// the recipients (`+0x11`..), a computer player's AI memory of it (`+0x7a8`'s `+0xbc00`),
    /// the sound and the minimap.
    pub(crate) fn flare(
        &mut self,
        target: Option<ObjectId>,
        point: Point,
        players: u8,
    ) -> Result<(), GenieError> {
        /// The flare's master (`0x112`, FLARE2).
        const FLARE: MasterId = MasterId::named(274);
        let target = ObjectId::field(target);
        let point = point.xy();
        if self.players.len() < usize::from(players) {
            return Ok(());
        }
        let at = if target == -1 {
            None
        } else {
            self.spatial.get(target).map(|p| p.position)
        };
        let at = at.unwrap_or_else(|| Position::new(point[0], point[1], 0.0));
        for p in PlayerId::each(usize::from(players)).skip(1) {
            self.remains(p, FLARE, at, None)?;
            self.made_new_object(p)?;
        }
        Ok(())
    }

    /// The flare a hit on player `p`'s object makes for it (`FUN_14120c0a0`: the player's slot
    /// 0x1c0 with master 112, `FUN_14162ce00` with the footprint left alone, as FLARE's,
    /// [`World::remains`] then [`World::made_new_object`]) at the attacker's place `at`; from a
    /// trebuchet (`doubled`) its amount (`+0x310`, which runs down as its time) doubles.
    ///
    /// # Errors
    ///
    /// What the new object does not model.
    pub(crate) fn make_flare(
        &mut self,
        p: PlayerId,
        at: Position,
        doubled: bool,
    ) -> Result<(), GenieError> {
        /// The flare a hit leaves (`0x70`, FLARE).
        const HIT_FLARE: MasterId = MasterId::named(112);
        let before = self.players[p.index()].objects.len();
        self.remains(p, HIT_FLARE, at, None)?;
        self.made_new_object(p)?;
        if doubled && let Some(f) = self.players[p.index()].objects.get_mut(before) {
            f.life.amount += f.life.amount;
        }
        Ok(())
    }

    /// A death spawn or the remains of a building master: its slot 0x40 (`FUN_14123d1a0`) makes a
    /// building at `at` (`FUN_1411ac5c0` with 1, so its init `FUN_1411de5f0`): the building's
    /// factory and constructor as any new building's ([`World::make_building`], with the unit AI
    /// its class gets, [`crate::objects::masters::Master::unit_ai_type`]). Not held: the dead
    /// object's `+0x6a8` and its facet (`FUN_14119d490`).
    ///
    /// # Errors
    ///
    /// A computer player's (its AI's slot 0x398), and what the building's making does not model.
    pub(super) fn new_building(
        &mut self,
        p: PlayerId,
        made: &crate::objects::masters::Master,
        at: Position,
    ) -> Result<(), GenieError> {
        if self.players[p.index()].computer {
            return Err(GenieError::unsupported(
                "a computer player's new building (its AI's slot 0x398)",
            ));
        }
        let ai_type = made.unit_ai_type(made.id);
        let _ = self.make_building(p, made.id, Ok(ai_type), at)?;
        Ok(())
    }
}
