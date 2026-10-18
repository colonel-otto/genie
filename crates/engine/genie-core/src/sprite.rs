//! An object's active sprite: the frame of its sprite (a `.dat` graphic) and, for a graphic that
//! animates, the clock that advances it. The save holds it (the object's active sprite list). An
//! attack lands on a frame of its sprite ([`crate::attack`]).
//!
//! The list's node factory (`FUN_1408dc940`) makes an animated node
//! (`RGE_Active_Animated_Sprite`, vtable `0x142f751b0`) for a graphic with sequence bit 0 and a
//! static one (`RGE_Active_Sprite`, vtable `0x142f75128`) for the others; a new sprite is a new
//! node, so the graphic tells the two apart.
//!
//! 158041, animated: start `FUN_1413b2ef0` (with `FUN_1413b3490`), update (slot 5)
//! `FUN_1413b5070`, the frame's reach test (slot 11) `FUN_1413b2d70`, the frame reset (slot 9)
//! `FUN_1413b4ff0`. Static: start `FUN_1413b3490`, update `FUN_1413b5330` (none), reach test
//! `FUN_1413b2d90` (always), frame reset `FUN_1413b5030`, looped (slot 12) `FUN_1413b38e0`
//! (never).

use crate::rng::Well512;

/// What the clock reads of a sprite (the `.dat` graphic, memory `+0x8`..`+0x1a`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Graphic {
    /// The graphic's id.
    pub id: i16,
    /// `+0x8`, `frame_count`.
    pub frames: i16,
    /// `+0xa`, `angle_count`: the facets.
    pub facets: i16,
    /// `+0xc`, `speed_multiplier`: at least 1e-7, it scales the frame duration.
    pub speed_multiplier: f32,
    /// `+0x10`, `frame_duration`.
    pub frame_duration: f32,
    /// `+0x14`, `replay_delay`.
    pub replay_delay: f32,
    /// `+0x1a`, `sequence_type`: bit 0 animated, 2 a random first frame, 3 played once, 4 backwards.
    pub flags: u8,
}

impl Graphic {
    const fn animated(&self) -> bool {
        self.flags & 1 != 0
    }

    const fn backwards(&self) -> bool {
        self.flags & 0x10 != 0
    }

    /// The first frame: the last one for a sprite played backwards.
    const fn first_frame(&self) -> i16 {
        if self.backwards() { self.frames - 1 } else { 0 }
    }
}

/// The shortest frame interval (`0x3a83126f`, also `DAT_142eb1e9c`).
const SHORTEST: f32 = crate::image::sprite::SHORTEST;
/// The gap below which a speed multiplier counts as none (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::sprite::EPSILON;

/// The active sprite (the save's `RGE_Active_Animated_Sprite`, memory `+0x24`..`+0x3b`; a static
/// one, the save's `RGE_Active_Sprite`, has only the frame, and the clock's fields stay at rest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveSprite {
    /// The graphic.
    pub graphic: Graphic,
    /// `+0x24`, `frame`.
    pub frame: i16,
    /// `+0x28`, `animate_interval`: one frame's time at the last speed.
    pub interval: f32,
    /// `+0x2c`, `animate_last`: the time since the last frame.
    pub last: f32,
    /// `+0x30`, `last_frame`: the frame before the last advance.
    pub last_frame: i16,
    /// `+0x34`, `frame_changed`; `+0x35`, `frame_looped`.
    pub changed: bool,
    /// `+0x35`, `frame_looped`.
    pub looped: bool,
    /// `+0x36`, `animate_flag`: 0 stopped, 1 playing, 2 in the replay delay.
    pub animate: u8,
    /// `+0x38`, `lastSpeedValue`.
    pub speed: f32,
}

impl ActiveSprite {
    /// A new sprite instance (`FUN_1413b2ef0`): its first frame and interval; a sprite with a
    /// random first frame draws it from the Sprite stream twice (`FUN_1413b3490`, then here). A
    /// static one ([`ActiveSprite::start_static`]) draws it once.
    pub fn start(graphic: Graphic, sprite_rng: &mut Well512) -> Self {
        if !graphic.animated() {
            return Self::start_static(graphic, sprite_rng);
        }
        let last = graphic.frames - 1;
        if graphic.flags & 4 != 0 {
            // FUN_1413b3490's draw, which the second one overrides
            let _ = sprite_rng.rand15();
        }
        let mut s = Self {
            graphic,
            frame: graphic.first_frame(),
            interval: graphic.frame_duration,
            last: 0.0,
            last_frame: graphic.first_frame(),
            changed: false,
            looped: false,
            animate: 1,
            speed: 0.0,
        };
        if f64::from(s.interval) < 0.001 {
            s.interval = SHORTEST;
        }
        if graphic.flags & 4 != 0 {
            let d = graphic.frame_duration;
            if 0.0 < d {
                let draw = sprite_rng.rand15();
                let t = (draw as i32 as f32
                    * (f32::from(graphic.frames) * d + graphic.replay_delay))
                    / 32767.0;
                let f = (t / d) as i32 as i16;
                s.frame = f;
                if graphic.frames <= f {
                    s.frame = last;
                    s.interval = t - f32::from(last) * d;
                }
            } else {
                let draw = sprite_rng.rand15() as i32;
                let f = (draw * i32::from(graphic.frames) / 0x7fff) as i16;
                s.frame = f.min(last);
            }
            s.last_frame = s.frame;
        }
        s
    }

    /// A static sprite instance (`FUN_1413b3490`): its first frame, or one the Sprite stream
    /// draws for a sprite with a random first frame.
    fn start_static(graphic: Graphic, sprite_rng: &mut Well512) -> Self {
        let last = graphic.frames - 1;
        let mut frame = graphic.first_frame();
        if graphic.flags & 4 != 0 {
            let draw = sprite_rng.rand15() as i32;
            frame = ((draw * i32::from(graphic.frames)) / 0x7fff) as i16;
            frame = frame.min(last);
        }
        Self::at_rest(graphic, frame)
    }

    /// A static sprite at `frame` (the save's `RGE_Active_Sprite`), its clock at rest.
    #[must_use]
    pub const fn at_rest(graphic: Graphic, frame: i16) -> Self {
        Self {
            graphic,
            frame,
            interval: 0.0,
            last: 0.0,
            last_frame: frame,
            changed: false,
            looped: false,
            animate: 0,
            speed: 0.0,
        }
    }

    /// Slot 9 with `0x7fff` (`FUN_1413b4ff0`): back to the first frame, not looped; a static
    /// sprite (`FUN_1413b5030`) only takes the frame.
    pub const fn reset_frame(&mut self) {
        if !self.graphic.animated() {
            self.frame = self.graphic.first_frame();
            return;
        }
        self.frame = self.graphic.first_frame();
        self.looped = false;
        self.last_frame = self.frame;
    }

    /// Slot 11 (`FUN_1413b2d70`): the last advance passed `frame` or looped; a static sprite
    /// (`FUN_1413b2d90`) has reached every frame.
    #[must_use]
    pub const fn reached(&self, frame: i16) -> bool {
        if !self.graphic.animated() {
            return true;
        }
        self.changed && (self.looped || (frame <= self.frame && self.last_frame < frame))
    }

    /// The interval of one frame at `speed` (at least 0.001).
    fn interval_at(&self, speed: f32) -> f32 {
        let g = &self.graphic;
        let mut i = g.frame_duration;
        if EPSILON <= g.speed_multiplier {
            i *= g.speed_multiplier;
        }
        if 0.0 < speed {
            i /= speed;
        }
        if i < SHORTEST { SHORTEST } else { i }
    }

    /// One update of `dt` seconds at the object's animation speed (`FUN_1413b5070`, the object's
    /// `+0x7b8`): an animated sprite's time runs, and its whole intervals advance it as many
    /// frames (backwards for such a sprite); past its last frame it loops, after its replay delay,
    /// or stops on it when played once.
    pub fn update(&mut self, dt: f32, speed: f32) {
        // a static sprite's update (`FUN_1413b5330`) does nothing
        if !self.graphic.animated() || self.animate == 0 {
            return;
        }
        self.changed = false;
        self.looped = false;
        let g = self.graphic;
        let last = g.frames - 1;
        let start = if self.frame > last {
            last
        } else if self.frame < 0 {
            0
        } else {
            self.frame
        };
        self.frame = start;
        if !g.animated() {
            return;
        }
        let time = dt + self.last;
        self.last = time;
        let interval = if self.speed == speed {
            self.interval
        } else {
            let i = self.interval_at(speed);
            self.interval = i;
            self.speed = speed;
            i
        };
        if time < interval {
            return;
        }
        let forward = !g.backwards();
        self.changed = true;
        self.last_frame = start;
        let steps = (time / interval) as i32 as i16;
        self.last = time - f32::from(steps) * interval;
        if 0.0 < speed || g.speed_multiplier == 0.0 {
            let f = start + if forward { steps } else { -steps };
            self.frame = f;
            self.looped = f < 0 || last < f;
        }
        self.interval = self.interval_at(speed);
        self.speed = speed;
        let mut stop = self.looped;
        if self.looped {
            let mut f;
            if g.replay_delay <= 0.0 || self.animate == 2 {
                self.animate = 1;
                f = self.frame % g.frames;
                if f < 0 {
                    f += g.frames;
                }
            } else {
                let mut at = self.frame;
                if at < 0 {
                    at = last - at;
                }
                stop = false;
                self.looped = false;
                self.animate = 2;
                self.interval = g.replay_delay - f32::from(at - g.frames) * g.frame_duration;
                f = if forward { 0 } else { last };
            }
            self.frame = f;
            if f == self.last_frame {
                let next = f + if forward { 1 } else { -1 };
                self.frame = next;
                if next < 0 || last < next {
                    stop = self.looped;
                    self.frame = if forward { 0 } else { last };
                }
            }
        }
        if stop && g.flags & 8 != 0 {
            self.animate = 0;
            self.frame = if forward { last.max(0) } else { 0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build 158041's Lumberjack Female (Attack), graphic 1239: 45 frames of 0.0444 s, animated.
    const CHOP: Graphic = Graphic {
        id: 1239,
        frames: 45,
        facets: 8,
        speed_multiplier: 0.0,
        frame_duration: 0.044_444_446,
        replay_delay: 0.0,
        flags: 3,
    };

    #[test]
    fn the_chop_reaches_its_frame_22_after_a_second_of_20_ms_updates() {
        // MarvinBlue/426048731's 9181 from update 1008 (the tick oracle's trace): frame 0 after
        // the first update (time 0.02), frame 1 after the third (time 0.0155...)
        let mut s = ActiveSprite::start(CHOP, &mut Well512::default());
        s.update(0.02, 1.0);
        assert_eq!((s.frame, s.changed), (0, false));
        s.update(0.02, 1.0);
        s.update(0.02, 1.0);
        assert_eq!(s.frame, 1);
        assert!((s.last - 0.015_555).abs() < 1e-5);
        let mut n = 3;
        while !s.reached(22) {
            s.update(0.02, 1.0);
            n += 1;
        }
        // update 1057, the 49th
        assert_eq!((n, s.frame), (49, 22));
    }

    #[test]
    fn a_static_sprite_stays_on_its_frame_and_has_reached_every_frame() {
        // build 158041's town center sprite 3241 does not animate (sequence bit 0 clear)
        let still = Graphic {
            id: 3241,
            frames: 1,
            facets: 1,
            speed_multiplier: 0.0,
            frame_duration: 0.0,
            replay_delay: 0.0,
            flags: 0,
        };
        let mut rng = Well512::default();
        let mut s = ActiveSprite::start(still, &mut rng);
        assert_eq!(rng, Well512::default());
        s.update(0.5, 1.0);
        assert_eq!((s.frame, s.changed, s.looped), (0, false, false));
        assert!(s.reached(0) && s.reached(7));
        // a random first frame draws once, not twice as an animated one does
        let mut once = Well512::default();
        let _ = once.rand15();
        let mut rng = Well512::default();
        let _ = ActiveSprite::start(Graphic { flags: 4, ..still }, &mut rng);
        assert_eq!(rng, once);
    }
}
