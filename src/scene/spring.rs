//! Springs for the scene's motion: critically damped, so they settle as fast as they can without
//! going past where they're headed, and they keep their speed when they're sent somewhere else on
//! the way, so a reversed morph turns around instead of starting over.

use std::f64::consts::TAU;

/// How long a spring takes to mostly settle, in seconds, at normal speed.
pub const RESPONSE: f64 = 0.42;

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

    /// Moves `dt` seconds on, `slowdown` times slower than normal, or straight to its target
    /// with `None`, as with animations off.
    pub fn advance(&mut self, dt: f64, slowdown: Option<f64>) {
        match slowdown.filter(|s| *s > 0.) {
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
