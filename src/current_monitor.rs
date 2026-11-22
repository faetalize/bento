//! The current monitor: the one that workspace actions and the shell's are about, and that new
//! windows open on.
//!
//! It's the monitor the pointer last came onto or was used on, or the one that last got the focus,
//! whichever was later. The keyboard's focus is apart from it: the pointer makes a monitor current
//! without taking the keyboard from the window it's in.

use smithay::output::Output;

#[derive(Debug, Default)]
pub struct CurrentMonitor {
    current: Option<Output>,
    /// The monitor the pointer is on.
    pointer: Option<Output>,
    /// The monitor that had the focus when this last heard.
    focused: Option<Output>,
}

impl CurrentMonitor {
    /// The current monitor, with the focus on `focused` now, which may have moved since this last
    /// heard.
    pub fn get<'a>(&'a self, focused: Option<&'a Output>) -> Option<&'a Output> {
        if self.focused.as_ref() != focused {
            return focused;
        }
        self.current.as_ref().or(focused)
    }

    /// The focus is on `focused` now. Only its moving to another monitor makes that current.
    pub fn focus_is_on(&mut self, focused: Option<&Output>) {
        if self.focused.as_ref() != focused {
            self.focused = focused.cloned();
            self.current = focused.cloned();
        }
    }

    /// The pointer is on `output`. Only its coming from another monitor makes that current, so
    /// that a mouse that's nudged doesn't.
    pub fn pointer_is_on(&mut self, output: &Output) {
        if self.pointer.as_ref() != Some(output) {
            self.make(output);
            self.pointer = Some(output.clone());
        }
    }

    /// The user did something on `output`, like a click, a scroll, or going to a window there.
    pub fn make(&mut self, output: &Output) {
        self.current = Some(output.clone());
    }

    /// `output` is gone.
    pub fn remove(&mut self, output: &Output) {
        for kept in [&mut self.current, &mut self.pointer, &mut self.focused] {
            if kept.as_ref() == Some(output) {
                *kept = None;
            }
        }
    }
}

/// The monitor that was current when bento started a program, kept with its activation token.
pub struct LaunchedOn(pub Output);
