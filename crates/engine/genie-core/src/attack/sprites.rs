//! The swing's and the arrival's sprites, the attack timer and the unit's turn to its target.

use crate::GenieError;
use crate::attack::{Arms, RELOAD_MS, Strike};
use crate::math::rng::Well512;
use crate::movement::Mover;
use crate::sprite::{ActiveSprite, Graphic};

/// The sprites a swing of a unit (its terrain look `look`, its master `master`) takes before its
/// Attack's own ([`SwingSprites`]): its look's attack sprite while it shows its look, its
/// master's special graphic.
#[must_use]
pub fn swing_sprites(
    look: Option<&crate::abilities::Look>,
    master: &crate::masters::Master,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
) -> SwingSprites {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    SwingSprites {
        look: look.and_then(|l| l.attacking).and_then(g),
        special: master
            .special_graphic
            .then(|| g(master.special_sprite))
            .flatten(),
    }
}

/// An Attack's draw (its hit's, its swing's second sprite) where the caller's walk holds no
/// Object stream (`Walk::plan`).
pub(super) const NO_OBJECT_STREAM: GenieError =
    GenieError::unsupported("a hit without the Object stream");

/// The sprites a swing takes before the Attack's own ([`Attack::set_swing`]).
///
/// [`Attack::set_swing`]: crate::attack::Attack::set_swing
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwingSprites {
    /// The unit's terrain look's attack sprite (`+0x970`, `FUN_14107cc20`) while it shows its
    /// look (`+0x8a9`).
    pub look: Option<Graphic>,
    /// Its master's special graphic (`+0x8e8`, a `TRIBE_Master_Combat_Object`'s).
    pub special: Option<Graphic>,
}

/// `FUN_141089820`: the attack timer becomes `t` and the reload world time the world's time plus
/// `t` in whole milliseconds (slot 0x248).
pub(super) fn set_attack_timer(s: &mut Strike<'_>, t: f32) {
    *s.attack_timer = t;
    *s.reload_worldtime = s.world_time.wrapping_add((t * RELOAD_MS) as i64 as u32);
}

/// `FUN_14120c0a0`'s test of `master`: it has a volley missile or a charge projectile its player
/// has (`masters`), so its hits go through its volley even without a missile.
pub(crate) fn volleys(master: &crate::masters::Master, masters: &crate::masters::Masters) -> bool {
    let has = |id: Option<i16>| id.and_then(|id| masters.get(id)).is_some();
    has(master.volley_missile) || has(master.charge_projectile)
}

/// Slot 0x68 (`FUN_141217000`): the unit's sprite becomes `g` unless it is already; a new sprite
/// starts a new instance (`FUN_1413b2ef0`).
/// [`Attack::arrived_sprite`] for a caller that holds the unit's sprite and animation speed: the
/// idle sprite of `arms` (slot 0x68), the speed `+0x7b8` cleared.
///
/// [`Attack::arrived_sprite`]: crate::attack::Attack::arrived_sprite
pub(crate) fn show_arrived(
    arms: &Arms,
    sprite: &mut Option<ActiveSprite>,
    anim_speed: &mut Option<f32>,
    rng: &mut Well512,
) -> Result<(), GenieError> {
    let g = arms.idle_sprite.ok_or(crate::turn::NO_STANDING_SPRITE)?;
    set_sprite(sprite, g, rng);
    if let Some(a) = anim_speed.as_mut() {
        *a = 0.0;
    }
    Ok(())
}

/// `FUN_1413b9f00` on the object's sprite list (`+0x148`, genie's: the unit's one sprite): the
/// first instance of graphic `g` (a node's `+0x8`) goes back to its first frame (its slot 9 with
/// `0x7fff`, [`ActiveSprite::reset_frame`]); false when the list holds none.
pub(crate) const fn reset_sprite(sprite: &mut Option<ActiveSprite>, g: Graphic) -> bool {
    match sprite.as_mut() {
        Some(sp) if sp.graphic.id == g.id => {
            sp.reset_frame();
            true
        }
        _ => false,
    }
}

pub(crate) fn set_sprite(sprite: &mut Option<ActiveSprite>, g: Graphic, rng: &mut Well512) {
    if sprite.is_some_and(|s| s.graphic.id == g.id) {
        return;
    }
    *sprite = Some(ActiveSprite::start(g, rng));
}

/// `FUN_141162db0`: the unit at `at` turns to face `to` with its sprites' facets, at once for a
/// unit without a turn time ([`crate::facing::face`]), else a facet a turn time
/// ([`crate::facing::turn_toward`] by the world time); returns whether it faces. The facets are
/// its current sprite's ([`Mover::facets`]).
pub(crate) fn face_target(
    mover: &mut Mover,
    at: [f32; 3],
    to: [f32; 3],
    world_time: u32,
) -> Result<bool, GenieError> {
    let facets = mover.facets().ok_or(GenieError::unsupported(
        "a unit facing a target whose sprites differ in their facets",
    ))?;
    let (at, to) = ([at[0], at[1]], [to[0], to[1]]);
    if mover.stats.turn_time == 0.0 {
        mover.angle = crate::facing::face(at, mover.angle, to, Some(facets));
        return Ok(true);
    }
    let (angle, last, faces) = crate::facing::turn_toward(
        at,
        mover.angle,
        to,
        Some(facets),
        mover.stats.turn_time,
        mover.last_turn,
        world_time,
    );
    (mover.angle, mover.last_turn) = (angle, last);
    Ok(faces)
}
