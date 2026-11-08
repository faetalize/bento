//! Springs for the shell's motion: critically damped, so they settle as fast as they can without
//! going past where they're headed, and they keep their speed when they're sent somewhere else
//! on the way, so a reversed morph turns around instead of starting over.

use std::cell::Cell;
use std::f64::consts::TAU;

/// How long a spring takes to mostly settle, in seconds, at normal speed.
pub const RESPONSE: f64 = 0.42;

thread_local! {
    /// How much slower than normal the shell moves, as bento's animations do, or `None` when
    /// they're off. It's one for the whole shell, like the setting it follows.
    static MOTION: Cell<Option<f64>> = const { Cell::new(Some(1.)) };
}

/// Sets how the shell moves: `Some` slowdown, or `None` for not at all.
pub fn set_motion(slowdown: Option<f64>) {
    MOTION.set(slowdown.filter(|slowdown| *slowdown > 0.));
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub value: f64,
    pub velocity: f64,
    pub target: f64,
}

impl Spring {
    pub fn new(value: f64) -> Self {
        Self {
            value,
            velocity: 0.,
            target: value,
        }
    }

    /// Goes straight to `value`, without moving.
    pub fn jump(&mut self, value: f64) {
        self.value = value;
        self.target = value;
        self.velocity = 0.;
    }

    /// Moves `dt` seconds on as the shell moves now: at its speed, or straight to its target when
    /// animations are off.
    pub fn advance(&mut self, dt: f64) {
        match MOTION.get() {
            Some(slowdown) => self.step(dt, RESPONSE * slowdown),
            None => self.jump(self.target),
        }
    }

    /// Moves `dt` seconds on. A critically damped spring has an exact solution, so a long frame
    /// can't make it overshoot or blow up.
    pub fn step(&mut self, dt: f64, response: f64) {
        let omega = TAU / response;
        let c1 = self.value - self.target;
        let c2 = self.velocity + omega * c1;
        let decay = (-omega * dt).exp();
        self.value = self.target + (c1 + c2 * dt) * decay;
        self.velocity = (c2 - omega * (c1 + c2 * dt)) * decay;
    }

    /// Whether it has as good as arrived, to within `epsilon` and barely moving, and if so puts it
    /// there.
    pub fn settle(&mut self, epsilon: f64) -> bool {
        let settled =
            (self.value - self.target).abs() < epsilon && self.velocity.abs() < epsilon * 10.;
        if settled {
            self.jump(self.target);
        }
        settled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_never_goes_past_its_target() {
        let mut spring = Spring::new(0.);
        spring.target = 100.;
        for _ in 0..200 {
            spring.step(1. / 60., RESPONSE);
            assert!(spring.value <= 100. + 1e-9, "{}", spring.value);
        }
        assert!(spring.settle(0.05));
    }

    #[test]
    fn with_animations_off_it_arrives_at_once_and_slowdown_slows_it() {
        let mut spring = Spring::new(0.);
        spring.target = 100.;
        set_motion(None);
        spring.advance(1. / 60.);
        assert_eq!(spring.value, 100.);

        let (mut normal, mut slow) = (Spring::new(0.), Spring::new(0.));
        normal.target = 100.;
        slow.target = 100.;
        set_motion(Some(1.));
        normal.advance(0.1);
        set_motion(Some(4.));
        slow.advance(0.1);
        assert!(slow.value < normal.value);
        set_motion(Some(1.));
    }

    #[test]
    fn a_long_frame_lands_it_rather_than_throwing_it() {
        let mut spring = Spring::new(0.);
        spring.target = 1.;
        spring.step(5., RESPONSE);
        assert!((spring.value - 1.).abs() < 1e-6);
    }
}
