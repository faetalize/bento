use niri_config::{Color, Gradient};
use smithay::utils::{Logical, Point, Rectangle, Size};

use super::props::{NodeTransform, Paint};
use crate::animation::{Animation, Clock};

/// A value that can be interpolated between two states.
pub trait Lerp {
    /// `t` is 0 at `from` and 1 at `to`. It leaves that range when a spring overshoots.
    fn lerp(from: &Self, to: &Self, t: f64) -> Self;
}

/// A property that animates to its new value when it's changed with a transition.
#[derive(Debug)]
pub struct Animated<T> {
    target: T,
    from: T,
    anim: Option<Animation>,
}

impl<T: Lerp + Clone + PartialEq> Animated<T> {
    pub fn new(value: T) -> Self {
        Self {
            from: value.clone(),
            target: value,
            anim: None,
        }
    }

    /// The value that is drawn right now.
    pub fn current(&self) -> T {
        match &self.anim {
            Some(anim) if !anim.is_done() => T::lerp(&self.from, &self.target, anim.value()),
            _ => self.target.clone(),
        }
    }

    /// Changes the value, animating from the drawn value if `animation` says how.
    pub fn set(&mut self, value: T, animation: Option<(&Clock, niri_config::Animation)>) {
        if value == self.target {
            return;
        }

        match animation {
            Some((clock, config)) if !config.off => {
                self.from = self.current();
                self.anim = Some(Animation::new(clock.clone(), 0., 1., 0., config));
            }
            _ => self.anim = None,
        }
        self.target = value;
    }

    /// Starts from `value` and animates to the current target, if `animation` says how. Without
    /// one, the value stays at the target.
    pub fn start_from(&mut self, value: T, animation: Option<(&Clock, niri_config::Animation)>) {
        if value == self.target {
            return;
        }

        if let Some((clock, config)) = animation.filter(|(_, config)| !config.off) {
            self.from = value;
            self.anim = Some(Animation::new(clock.clone(), 0., 1., 0., config));
        }
    }

    pub fn advance(&mut self) {
        if self.anim.as_ref().is_some_and(|anim| anim.is_done()) {
            self.anim = None;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.anim.is_some()
    }
}

impl Lerp for f64 {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        from + (to - from) * t
    }
}

impl Lerp for Rectangle<f64, Logical> {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        Rectangle::new(
            Point::from((
                f64::lerp(&from.loc.x, &to.loc.x, t),
                f64::lerp(&from.loc.y, &to.loc.y, t),
            )),
            Size::from((
                f64::lerp(&from.size.w, &to.size.w, t).max(0.),
                f64::lerp(&from.size.h, &to.size.h, t).max(0.),
            )),
        )
    }
}

impl Lerp for NodeTransform {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        NodeTransform {
            scale: f64::lerp(&from.scale, &to.scale, t).max(0.),
            x: f64::lerp(&from.x, &to.x, t),
            y: f64::lerp(&from.y, &to.y, t),
        }
    }
}

/// Mixes premultiplied, like CSS.
fn mix_colors(from: Color, to: Color, t: f64) -> Color {
    let t = t as f32;
    let from = from.to_array_premul();
    let to = to.to_array_premul();
    let mix = |i: usize| (from[i] + (to[i] - from[i]) * t).clamp(0., 1.);
    Color::from_array_premul([mix(0), mix(1), mix(2), mix(3)])
}

/// Mixes two paints. A colour mixes with a gradient as a gradient of that one colour, which takes
/// the other's direction, so that only the colours change. Gradients mix their colours and angle,
/// and the second one's area and colour space apply.
fn mix_paints(from: Paint, to: Paint, t: f64) -> Paint {
    let (a, b) = match (from, to) {
        (Paint::Color(from), Paint::Color(to)) => return Paint::Color(mix_colors(from, to, t)),
        (Paint::Gradient(a), Paint::Gradient(b)) => (a, b),
        (Paint::Color(color), Paint::Gradient(b)) => (
            Gradient {
                from: color,
                to: color,
                ..b
            },
            b,
        ),
        (Paint::Gradient(a), Paint::Color(color)) => (
            a,
            Gradient {
                from: color,
                to: color,
                ..a
            },
        ),
    };

    let angle = f64::lerp(&f64::from(a.angle), &f64::from(b.angle), t).round();
    Paint::Gradient(Gradient {
        from: mix_colors(a.from, b.from, t),
        to: mix_colors(a.to, b.to, t),
        angle: angle.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16,
        relative_to: b.relative_to,
        in_: b.in_,
    })
}

impl Lerp for Option<Paint> {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        match (*from, *to) {
            (None, None) => None,
            (Some(from), Some(to)) => Some(mix_paints(from, to, t)),
            // No paint is the other paint, fully transparent.
            (Some(from), None) => Some(mix_paints(from, from.transparent(), t)),
            (None, Some(to)) => Some(mix_paints(to.transparent(), to, t)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use niri_config::animations::{Curve, EasingParams, Kind};

    use super::*;

    fn linear(duration_ms: u32) -> niri_config::Animation {
        niri_config::Animation {
            off: false,
            kind: Kind::Easing(EasingParams {
                duration_ms,
                curve: Curve::Linear,
            }),
        }
    }

    #[test]
    fn numbers_interpolate_and_extrapolate() {
        assert_eq!(f64::lerp(&10., &20., 0.), 10.);
        assert_eq!(f64::lerp(&10., &20., 0.25), 12.5);
        assert_eq!(f64::lerp(&10., &20., 1.), 20.);
        // A spring can overshoot.
        assert_eq!(f64::lerp(&10., &20., 1.5), 25.);
    }

    #[test]
    fn rects_interpolate_and_keep_their_size_non_negative() {
        let from = Rectangle::new(Point::from((0., 0.)), Size::from((100., 50.)));
        let to = Rectangle::new(Point::from((100., 20.)), Size::from((200., 50.)));
        let mid = Rectangle::lerp(&from, &to, 0.5);
        assert_eq!(mid.loc, Point::from((50., 10.)));
        assert_eq!(mid.size, Size::from((150., 50.)));

        let shrunk = Rectangle::lerp(&to, &from, 3.);
        assert!(shrunk.size.w >= 0. && shrunk.size.h >= 0.);
    }

    #[test]
    fn transforms_interpolate() {
        let from = NodeTransform::IDENTITY;
        let to = NodeTransform {
            scale: 2.,
            x: 10.,
            y: -10.,
        };
        let mid = NodeTransform::lerp(&from, &to, 0.5);
        assert_eq!(
            mid,
            NodeTransform {
                scale: 1.5,
                x: 5.,
                y: -5.
            }
        );
    }

    #[test]
    fn colors_mix_premultiplied_and_stay_in_range() {
        let black = Color::from_rgba8_unpremul(0, 0, 0, 255);
        let white = Color::from_rgba8_unpremul(255, 255, 255, 255);
        let mid = mix_colors(black, white, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-6 && (mid.a - 1.).abs() < 1e-6);

        let over = mix_colors(black, white, 1.5);
        assert!(over.r <= 1. && over.a <= 1.);
    }

    #[test]
    fn a_missing_paint_is_transparent() {
        let red = Color::from_rgba8_unpremul(255, 0, 0, 255);
        let paint = Some(Paint::Color(red));
        assert_eq!(Option::<Paint>::lerp(&None, &None, 0.5), None);

        let Some(Paint::Color(half)) = Option::<Paint>::lerp(&None, &paint, 0.5) else {
            panic!();
        };
        assert!((half.a - 0.5).abs() < 1e-6);
        assert!((half.r - 1.).abs() < 1e-6);

        let Some(Paint::Color(gone)) = Option::<Paint>::lerp(&paint, &None, 1.) else {
            panic!();
        };
        assert_eq!(gone.a, 0.);
    }

    fn gradient(from: Color, to: Color, angle: i16) -> Gradient {
        Gradient {
            from,
            to,
            angle,
            ..Gradient::from(from)
        }
    }

    #[test]
    fn gradients_mix_their_colors_and_angle() {
        let black = Color::from_rgba8_unpremul(0, 0, 0, 255);
        let white = Color::from_rgba8_unpremul(255, 255, 255, 255);
        let from = Paint::Gradient(gradient(black, black, 0));
        let to = Paint::Gradient(gradient(white, white, 90));

        let Paint::Gradient(mid) = mix_paints(from, to, 0.5) else {
            panic!();
        };
        assert!((mid.from.r - 0.5).abs() < 1e-6 && (mid.to.r - 0.5).abs() < 1e-6);
        assert_eq!(mid.angle, 45);
    }

    #[test]
    fn a_colour_mixes_with_a_gradient_as_a_flat_gradient_of_its_direction() {
        let black = Color::from_rgba8_unpremul(0, 0, 0, 255);
        let white = Color::from_rgba8_unpremul(255, 255, 255, 255);
        let red = Color::from_rgba8_unpremul(255, 0, 0, 255);
        let target = gradient(white, red, 270);

        // The direction is the gradient's from the start, so only the colours move.
        let Paint::Gradient(start) = mix_paints(Paint::Color(black), Paint::Gradient(target), 0.)
        else {
            panic!();
        };
        assert_eq!(start.from, black);
        assert_eq!(start.to, black);
        assert_eq!(start.angle, 270);

        let Paint::Gradient(end) = mix_paints(Paint::Gradient(target), Paint::Color(black), 1.)
        else {
            panic!();
        };
        assert_eq!(end.from, black);
        assert_eq!(end.to, black);
        assert_eq!(end.angle, 270);

        // Two colours stay a colour.
        assert!(matches!(
            mix_paints(Paint::Color(black), Paint::Color(white), 0.5),
            Paint::Color(_)
        ));
    }

    #[test]
    fn a_change_without_a_transition_is_immediate() {
        let mut value = Animated::new(1.);
        value.set(5., None);
        assert_eq!(value.current(), 5.);
        assert!(!value.is_animating());
    }

    #[test]
    fn a_change_with_a_transition_starts_from_the_drawn_value() {
        let mut clock = Clock::with_time(Duration::ZERO);
        let mut value = Animated::new(0.);

        value.set(100., Some((&clock, linear(100))));
        assert!(value.is_animating());
        assert_eq!(value.current(), 0.);

        clock.set_unadjusted(Duration::from_millis(50));
        assert_eq!(value.current(), 50.);

        // Changing it again mid-flight continues from where it's drawn.
        value.set(0., Some((&clock, linear(100))));
        assert_eq!(value.current(), 50.);
        clock.set_unadjusted(Duration::from_millis(100));
        assert_eq!(value.current(), 25.);

        clock.set_unadjusted(Duration::from_millis(150));
        assert_eq!(value.current(), 0.);
        value.advance();
        assert!(!value.is_animating());
    }

    #[test]
    fn starting_from_a_value_animates_to_the_target() {
        let mut clock = Clock::with_time(Duration::ZERO);
        let mut value = Animated::new(10.);

        value.start_from(0., Some((&clock, linear(100))));
        assert!(value.is_animating());
        assert_eq!(value.current(), 0.);

        clock.set_unadjusted(Duration::from_millis(50));
        assert_eq!(value.current(), 5.);
        clock.set_unadjusted(Duration::from_millis(100));
        assert_eq!(value.current(), 10.);
    }

    #[test]
    fn starting_from_a_value_without_a_transition_stays_at_the_target() {
        let clock = Clock::with_time(Duration::ZERO);
        let mut value = Animated::new(10.);

        value.start_from(0., None);
        assert_eq!(value.current(), 10.);
        assert!(!value.is_animating());

        let mut off = linear(100);
        off.off = true;
        value.start_from(0., Some((&clock, off)));
        assert_eq!(value.current(), 10.);
        assert!(!value.is_animating());

        // Starting where it is changes nothing.
        value.start_from(10., Some((&clock, linear(100))));
        assert!(!value.is_animating());
    }

    #[test]
    fn setting_the_same_value_does_not_restart() {
        let mut clock = Clock::with_time(Duration::ZERO);
        let mut value = Animated::new(0.);
        value.set(100., Some((&clock, linear(100))));
        clock.set_unadjusted(Duration::from_millis(50));

        value.set(100., Some((&clock, linear(100))));
        assert_eq!(value.current(), 50.);
    }

    #[test]
    fn a_transition_that_is_off_is_immediate() {
        let clock = Clock::with_time(Duration::ZERO);
        let mut value = Animated::new(0.);
        let mut config = linear(100);
        config.off = true;

        value.set(100., Some((&clock, config)));
        assert_eq!(value.current(), 100.);
        assert!(!value.is_animating());
    }
}
