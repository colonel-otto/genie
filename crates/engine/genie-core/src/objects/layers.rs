//! The object's class chain (FD-045-D06): the game's object classes, each layer holding the one
//! below it, as the save reads them (`RgeMovingObject` holds `rge_animated_object`).
//!
//! A part of an object (its mover, an action, its unit AI) reaches its owner's layer through one
//! borrow, and a call the game makes on its own object is a method of that layer: the mover's
//! stop (slot 0x5d0) clears the animated layer's speed at once ([`Mover::stop`]). The layers below
//! the action layer an object has are its [`Body`].

use crate::movement::Mover;

/// `RGE_Animated_Object`'s own field (types 20 and up): `mCurrentSpeed` (`+0x7b8`), the speed the
/// object's sprite clock runs at. Its constructor (`FUN_141070cf0`) sets it to 0; a moving
/// unit's velocity sets it as it walks, and a stop clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Animated {
    /// `mCurrentSpeed` (`+0x7b8`).
    pub speed: f32,
}

/// The layers of an object below its action layer, by its type: the static object alone (types
/// below 20), the animated object (20 to 29), or the moving object (30 and up), which holds the
/// animated one.
#[derive(Clone, Debug, Default, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "nearly every object moves; a box per unit would cost more than the static few save"
)]
pub enum Body {
    /// A static object (types below 20): no animation speed, no movement.
    #[default]
    Static,
    /// An animated object that does not move (types 20 to 29).
    Animated(Animated),
    /// A moving object (types 30 and up), its animated layer inside its mover.
    Moving(Mover),
}

impl Body {
    /// The layers of an object with `animated` and `mover` (each None where its type has none): a
    /// mover holds the animated layer (`animated`, else the mover's own), else the animated layer
    /// stands alone, else the object is static.
    #[must_use]
    pub fn new(animated: Option<Animated>, mover: Option<Mover>) -> Self {
        match (animated, mover) {
            (a, Some(mut m)) => {
                if let Some(a) = a {
                    m.animated = a;
                }
                Self::Moving(m)
            }
            (Some(a), None) => Self::Animated(a),
            (None, None) => Self::Static,
        }
    }

    /// The moving layer, if the object moves.
    #[must_use]
    pub const fn mover(&self) -> Option<&Mover> {
        match self {
            Self::Moving(m) => Some(m),
            Self::Static | Self::Animated(_) => None,
        }
    }

    /// The moving layer to change, if the object moves.
    pub const fn mover_mut(&mut self) -> Option<&mut Mover> {
        match self {
            Self::Moving(m) => Some(m),
            Self::Static | Self::Animated(_) => None,
        }
    }

    /// The animated layer, if the object has one: its own, or its mover's.
    #[must_use]
    pub const fn animated(&self) -> Option<&Animated> {
        match self {
            Self::Animated(a) | Self::Moving(Mover { animated: a, .. }) => Some(a),
            Self::Static => None,
        }
    }

    /// The animated layer to change, if the object has one.
    pub const fn animated_mut(&mut self) -> Option<&mut Animated> {
        match self {
            Self::Animated(a) | Self::Moving(Mover { animated: a, .. }) => Some(a),
            Self::Static => None,
        }
    }

    /// The animation speed (`+0x7b8`), None for a static object.
    #[must_use]
    pub fn anim_speed(&self) -> Option<f32> {
        self.animated().map(|a| a.speed)
    }

    /// Sets the animation speed (`+0x7b8`) of an object that has one; a static object has none.
    pub const fn set_anim_speed(&mut self, speed: f32) {
        if let Some(a) = self.animated_mut() {
            a.speed = speed;
        }
    }

    /// The object's mover goes and its animated layer stays (a caller that works on the mover
    /// apart from its object puts it back with [`Body::put_mover`]).
    pub fn take_mover(&mut self) -> Option<Mover> {
        match std::mem::take(self) {
            Self::Moving(m) => {
                *self = Self::Animated(m.animated);
                Some(m)
            }
            other => {
                *self = other;
                None
            }
        }
    }

    /// A mover taken with [`Body::take_mover`] comes back, with the animated layer as it is now
    /// (a static body takes the mover's own).
    pub fn put_mover(&mut self, mover: Option<Mover>) {
        let animated = self.animated().copied();
        *self = Self::new(animated, mover);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mover() -> Mover {
        crate::movement::tests::builder_9177()
    }

    #[test]
    fn a_body_holds_the_layers_its_type_has() {
        let a = Animated { speed: 0.5 };
        assert_eq!(Body::new(None, None), Body::Static);
        assert_eq!(Body::new(Some(a), None), Body::Animated(a));
        let moving = Body::new(Some(a), Some(mover()));
        assert_eq!(moving.mover().map(|m| m.animated), Some(a));
        assert_eq!(moving.anim_speed(), Some(0.5));
        let mut own = mover();
        own.animated.speed = 0.25;
        assert_eq!(
            Body::new(None, Some(own)).anim_speed(),
            Some(0.25),
            "a mover always has an animated layer, its own without another"
        );
    }

    #[test]
    fn only_a_static_object_has_no_animation_speed() {
        let mut still = Body::Static;
        still.set_anim_speed(1.0);
        assert_eq!((still.anim_speed(), still.mover()), (None, None));
        let mut flag = Body::Animated(Animated::default());
        flag.set_anim_speed(1.0);
        assert_eq!((flag.anim_speed(), flag.mover()), (Some(1.0), None));
        let mut walker = Body::new(None, Some(mover()));
        walker.set_anim_speed(2.0);
        assert_eq!(walker.mover_mut().map(|m| m.animated.speed), Some(2.0));
        assert_eq!(walker.animated_mut().map(|a| a.speed), Some(2.0));
    }

    #[test]
    fn a_taken_mover_leaves_the_animated_layer_and_comes_back_with_it() {
        let mut body = Body::new(Some(Animated { speed: 1.0 }), Some(mover()));
        let taken = body.take_mover();
        assert_eq!(body, Body::Animated(Animated { speed: 1.0 }));
        body.set_anim_speed(0.0);
        body.put_mover(taken);
        assert_eq!(body.mover().map(|m| m.animated.speed), Some(0.0));
        let mut still = Body::Static;
        assert_eq!(still.take_mover(), None);
        assert_eq!(still, Body::Static);
    }
}
