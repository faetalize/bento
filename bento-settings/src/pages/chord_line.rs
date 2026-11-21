//! A row's chords on one line, a dot between each. Those that don't fit leave a "+N" at the end
//! instead, counting them, so a row stays one line of chords however many it has.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// The room on each side of a dot, and before the "+N".
const GAP: i32 = 8;

glib::wrapper! {
    pub struct ChordLine(ObjectSubclass<imp::ChordLine>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ChordLine {
    pub fn new(chords: Vec<gtk::Widget>) -> Self {
        let line: Self = glib::Object::new();
        let dim = |text: &str| -> gtk::Widget {
            gtk::Label::builder()
                .label(text)
                .valign(gtk::Align::Center)
                .css_classes(["dim-label", "numeric"])
                .build()
                .upcast()
        };
        let count = chords.len();
        let dots: Vec<gtk::Widget> = (1..count).map(|_| dim("·")).collect();
        let more: Vec<gtk::Widget> = (1..count).map(|n| dim(&format!("+{n}"))).collect();
        for widget in chords.iter().chain(&dots).chain(&more) {
            widget.set_parent(&line);
        }
        let imp = line.imp();
        imp.chords.replace(chords);
        imp.dots.replace(dots);
        imp.more.replace(more);
        line
    }
}

fn width(widget: &gtk::Widget) -> i32 {
    widget.measure(gtk::Orientation::Horizontal, -1).1
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    pub struct ChordLine {
        pub chords: RefCell<Vec<gtk::Widget>>,
        pub dots: RefCell<Vec<gtk::Widget>>,
        /// "+1", "+2"…: the one for how many chords don't fit shows.
        pub more: RefCell<Vec<gtk::Widget>>,
    }

    impl ChordLine {
        /// The width of the first `k` chords with the dots between them.
        fn run(&self, k: usize) -> i32 {
            let chords: i32 = self.chords.borrow()[..k].iter().map(width).sum();
            let dots: i32 = self.dots.borrow()[..k.saturating_sub(1)]
                .iter()
                .map(width)
                .sum();
            chords + dots + 2 * GAP * k.saturating_sub(1) as i32
        }

        /// How many chords show in `available`: all of them, or as many as leave room for the
        /// "+N", but always the first.
        fn shown(&self, available: i32) -> usize {
            let count = self.chords.borrow().len();
            if self.run(count) <= available {
                return count;
            }
            let more = self.more.borrow();
            (1..count)
                .rev()
                .find(|&k| self.run(k) + GAP + width(&more[count - k - 1]) <= available)
                .unwrap_or(1)
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChordLine {
        const NAME: &'static str = "BentoChordLine";
        type Type = super::ChordLine;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ChordLine {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ChordLine {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Vertical {
                let (mut min, mut nat) = (0, 0);
                let mut child = self.obj().first_child();
                while let Some(c) = child {
                    let (m, n, ..) = c.measure(gtk::Orientation::Vertical, -1);
                    (min, nat) = (min.max(m), nat.max(n));
                    child = c.next_sibling();
                }
                return (min, nat, -1, -1);
            }
            // At least the first chord and the widest "+N", at best every chord.
            let first = self.chords.borrow().first().map_or(0, width);
            let more = self.more.borrow().iter().map(width).max();
            let min = first + more.map_or(0, |w| w + GAP);
            let count = self.chords.borrow().len();
            (min, self.run(count).max(min), -1, -1)
        }

        fn size_allocate(&self, available: i32, height: i32, _baseline: i32) {
            let (chords, dots, more) =
                (self.chords.borrow(), self.dots.borrow(), self.more.borrow());
            let count = chords.len();
            let k = self.shown(available);
            let mut x = 0;
            let place = |child: &gtk::Widget, x: &mut i32| {
                let w = width(child);
                let h = child.measure(gtk::Orientation::Vertical, w).1;
                child.set_child_visible(true);
                let at = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(
                    *x as f32,
                    ((height - h) / 2) as f32,
                ));
                child.allocate(w, h, -1, Some(at));
                *x += w;
            };
            for (i, chord) in chords.iter().enumerate() {
                if i >= k {
                    chord.set_child_visible(false);
                    continue;
                }
                if i > 0 {
                    x += GAP;
                    place(&dots[i - 1], &mut x);
                    x += GAP;
                }
                place(chord, &mut x);
            }
            for dot in dots.iter().skip(k.saturating_sub(1)) {
                dot.set_child_visible(false);
            }
            for (i, label) in more.iter().enumerate() {
                if k < count && i == count - k - 1 {
                    x += GAP;
                    place(label, &mut x);
                } else {
                    label.set_child_visible(false);
                }
            }
        }
    }
}
