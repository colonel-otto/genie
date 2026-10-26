//! The values the slice derives from a master after a change ([`Master::follow`]): the
//! sprites' speed and facets, a moving unit's values.

use crate::movement::MoveStats;
use crate::objects::masters::{Master, master_type};

/// A sprite's speed factor (the `.dat` graphic's `speed_multiplier`): the master's speed times
/// it when above 0, else the master's speed (as the masters' loader derives
/// [`MoveStats`]' speeds).
pub(super) fn sprite_speed(
    base: f32,
    sprite: i16,
    graphics: &std::collections::BTreeMap<i16, crate::objects::sprite::Graphic>,
) -> f32 {
    let factor = graphics.get(&sprite).map_or(0.0, |g| g.speed_multiplier);
    if 0.0 < factor { base * factor } else { base }
}

impl Master {
    /// The facets the facing routines read of a unit of this master ([`MoveStats::facets`]):
    /// those of its standing sprites, its moving ones (types 30 and up), its attack sprite (50
    /// and up) and its tasks' sprites, when every one the graphics hold has the same count and
    /// bit 1 (as the masters' loader derives them).
    #[must_use]
    pub fn sprite_facets(
        &self,
        graphics: &std::collections::BTreeMap<i16, crate::objects::sprite::Graphic>,
    ) -> Option<crate::objects::facing::SpriteFacets> {
        let mut sprites = vec![self.standing_sprite, self.standing_sprite_2.unwrap_or(-1)];
        if master_type::MOVING <= self.kind {
            sprites.extend([self.move_sprite, self.run_sprite]);
        }
        if master_type::RGE_COMBAT <= self.kind {
            sprites.push(self.attack_sprite);
        }
        for t in &self.tasks {
            sprites.extend(t.sprites);
        }
        let mut seen = sprites
            .into_iter()
            .filter(|&id| 0 <= id)
            .filter_map(|id| graphics.get(&id))
            .map(|g| crate::objects::facing::SpriteFacets {
                count: g.facets,
                flag_1: g.flags & 2 != 0,
            });
        let first = seen.next()?;
        seen.all(|f| f == first).then_some(first)
    }

    /// What a unit moving with this master reads after attribute `attribute` changed
    /// (`stats`, the master's [`Master::moving`] or a unit's copy): in the game the unit reads its
    /// master, so its radii, speeds, turn time, terrain restriction (its mask from `terrain`,
    /// kept without one), fast turn, weapon range and facets follow the master's values.
    pub fn follow_moving(
        &self,
        stats: &mut MoveStats,
        attribute: u8,
        graphics: &std::collections::BTreeMap<i16, crate::objects::sprite::Graphic>,
        terrain: Option<&crate::map::terrain::Terrain>,
    ) {
        use crate::objects::masters::attribute as a;
        match attribute {
            a::SIZE_X | a::SIZE_Y | a::SIZE_Z => {
                stats.radius = [self.radius[0], self.radius[1], self.height];
            }
            a::MOVEMENT_SPEED | a::STANDING_GRAPHIC | a::WALKING_GRAPHIC | a::RUNNING_GRAPHIC => {
                let run = if 0 <= self.run_sprite {
                    self.run_sprite
                } else {
                    self.move_sprite
                };
                stats.speed = sprite_speed(self.max_speed, run, graphics);
                stats.run_speed = stats.speed;
                stats.walk_speed = sprite_speed(self.max_speed, self.move_sprite, graphics);
                stats.stand_speed = sprite_speed(self.max_speed, self.standing_sprite, graphics);
            }
            a::ROTATION_SPEED => stats.turn_time = self.turn_speed,
            a::TERRAIN_RESTRICTION_ID => {
                // a restriction below 0 has no mask; the stats keep theirs
                if let Ok(r) = usize::try_from(self.terrain_restriction) {
                    stats.restriction = r;
                    if let Some(t) = terrain {
                        stats.restriction_mask = t.restriction_mask(r);
                    }
                }
            }
            a::UNIT_TRAIT | a::TRAIT_PIECE => stats.fast_turn = self.object_flags & 2 != 0,
            a::MAXIMUM_RANGE => stats.weapon_range = self.weapon_range,
            _ => {}
        }
        if matches!(
            attribute,
            a::STANDING_GRAPHIC
                | a::STANDING_GRAPHIC_2
                | a::WALKING_GRAPHIC
                | a::RUNNING_GRAPHIC
                | a::ATTACK_GRAPHIC
        ) {
            stats.facets = self.sprite_facets(graphics);
        }
    }

    /// The values the slice derives from this master's after attribute `attribute` changed: its
    /// height test, its missile's flight, whether it has damage sprites and a special graphic,
    /// its train locations' ids and first time, and what a unit moving with it reads
    /// ([`Master::follow_moving`]).
    pub fn follow(
        &mut self,
        attribute: u8,
        graphics: &std::collections::BTreeMap<i16, crate::objects::sprite::Graphic>,
        terrain: Option<&crate::map::terrain::Terrain>,
    ) {
        use crate::objects::masters::attribute as a;
        match attribute {
            a::SIZE_X | a::SIZE_Y | a::SIZE_Z => {
                self.tall = 0.0 < self.height;
                if let Some(m) = self.missile_stats.as_mut() {
                    m.radius = [self.radius[0], self.radius[1], self.height];
                }
            }
            a::MOVEMENT_SPEED => {
                if let Some(m) = self.missile_stats.as_mut() {
                    m.speed = self.max_speed;
                }
            }
            a::DAMAGE_GRAPHICS_ENTRY_MOD | a::DAMAGE_GRAPHICS_TOTAL_NUM => {
                self.damage_sprites = !self.damage_graphics.is_empty();
            }
            a::SPECIAL_GRAPHIC => self.special_graphic = 0 <= self.special_sprite,
            a::TRAIN_LOCATION
            | a::TRAIN_TIME
            | a::TRAIN_LOCATIONS_ENTRY_MOD
            | a::TRAIN_LOCATIONS_TOTAL_NUM => {
                self.trained_at = self.train_locations.iter().map(|t| t.location).collect();
                self.build_time = self.train_locations.first().map_or(0, |t| t.time);
            }
            _ => {}
        }
        if let Some(mut stats) = self.moving.take() {
            self.follow_moving(&mut stats, attribute, graphics, terrain);
            self.moving = Some(stats);
        }
    }
}
