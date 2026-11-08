//! The stage: a surface's content as pieces of glass. A piece grows out of a spot, like a bar
//! item, into its own place, and shrinks back into it when it closes. Each frame the stage draws
//! the pieces' content, and gives bento the same shapes as glass and as input region. bento shades
//! the glass itself, so the content goes straight on it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{graphene, gsk};

use crate::wayland::{Glass, Shape};

/// How long a piece takes to grow, and to shrink back.
const OPEN_MS: u32 = 320;
const CLOSE_MS: u32 = 220;

/// Where a piece sits once open, and where it grows from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn lerp(self, to: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self::new(
            mix(self.x, to.x),
            mix(self.y, to.y),
            mix(self.width, to.width),
            mix(self.height, to.height),
        )
    }

    fn graphene(self) -> graphene::Rect {
        graphene::Rect::new(self.x, self.y, self.width, self.height)
    }
}

/// Lays out a piece for the stage's width: from where it grows, to where it sits.
pub type Place = Rc<dyn Fn(&Stage, f32) -> (Rect, Rect)>;

struct Piece {
    child: gtk::Widget,
    place: Place,
    radius: f32,
    /// 0 when it's shut into its source, 1 when open.
    progress: Rc<Cell<f64>>,
    animation: Option<adw::TimedAnimation>,
    open: bool,
    from: Rect,
    to: Rect,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Stage {
        pub(super) pieces: RefCell<Vec<Piece>>,
        pub glass: RefCell<Option<Glass>>,
        pub on_resize: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Stage {
        const NAME: &'static str = "BentoShellStage";
        type Type = super::Stage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Stage {
        fn dispose(&self) {
            for piece in self.pieces.take() {
                piece.child.unparent();
            }
        }
    }

    impl WidgetImpl for Stage {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let obj = self.obj();
            let width = if for_size >= 0 { for_size } else { obj.width() } as f32;
            // Only showing pieces take room, so the surface shrinks when they've closed.
            let showing: Vec<Place> = self
                .pieces
                .borrow()
                .iter()
                .filter(|piece| piece.open || piece.progress.get() > 0.)
                .map(|piece| piece.place.clone())
                .collect();
            let size = showing
                .iter()
                .map(|place| {
                    let (_, to) = place(&obj, width);
                    match orientation {
                        gtk::Orientation::Horizontal => to.width,
                        _ => to.y + to.height,
                    }
                })
                .fold(0f32, f32::max)
                .ceil() as i32;
            (size, size, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let obj = self.obj();
            let places: Vec<Place> = self
                .pieces
                .borrow()
                .iter()
                .map(|piece| piece.place.clone())
                .collect();
            let placed: Vec<(Rect, Rect)> = places
                .iter()
                .map(|place| place(&obj, width as f32))
                .collect();

            let mut pieces = self.pieces.borrow_mut();
            for (piece, (from, to)) in pieces.iter_mut().zip(placed) {
                piece.from = from;
                piece.to = to;
                piece.child.size_allocate(
                    &gtk::Allocation::new(
                        to.x.round() as i32,
                        to.y.round() as i32,
                        to.width.round() as i32,
                        to.height.round() as i32,
                    ),
                    -1,
                );
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let mut shapes = Vec::new();

            for piece in self.pieces.borrow().iter() {
                let t = piece.progress.get() as f32;
                if t <= 0. {
                    continue;
                }

                let rect = piece.from.lerp(piece.to, t);
                let radius = piece.radius.min(rect.width / 2.).min(rect.height / 2.);
                shapes.push(Shape {
                    x: rect.x.round() as i32,
                    y: rect.y.round() as i32,
                    width: rect.width.round() as i32,
                    height: rect.height.round() as i32,
                    radius: radius.round() as i32,
                });

                let clip = gsk::RoundedRect::from_rect(rect.graphene(), radius);
                snapshot.push_rounded_clip(&clip);
                // The content shows once the piece is mostly grown.
                let shown = ((t - 0.35) / 0.65).clamp(0., 1.);
                if shown > 0. {
                    snapshot.push_opacity(shown as f64);
                    obj.snapshot_child(&piece.child, snapshot);
                    snapshot.pop();
                }
                snapshot.pop();
            }

            if let Some(glass) = &*self.glass.borrow() {
                glass.set_shapes(&shapes, &[]);
            }
            obj.set_input(&shapes);
        }
    }
}

glib::wrapper! {
    pub struct Stage(ObjectSubclass<imp::Stage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Stage {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Stage {
    /// The glass behind the stage's surface.
    pub fn set_glass(&self, glass: Option<Glass>) {
        self.imp().glass.replace(glass);
        self.queue_draw();
    }

    /// Called when the stage needs a different size, as pieces open and close.
    pub fn connect_resize(&self, f: impl Fn() + 'static) {
        self.imp().on_resize.replace(Some(Box::new(f)));
    }

    /// Adds a piece that's already open, like the bar itself.
    pub fn add_fixed(&self, child: &impl IsA<gtk::Widget>, radius: f32, place: Place) {
        self.add(child.as_ref(), radius, place, 1.);
    }

    /// Adds a piece, shut, so `open` grows it.
    pub fn add_piece(&self, child: &impl IsA<gtk::Widget>, radius: f32, place: Place) {
        self.add(child.as_ref(), radius, place, 0.);
    }

    fn add(&self, child: &gtk::Widget, radius: f32, place: Place, progress: f64) {
        child.set_parent(self);
        self.imp().pieces.borrow_mut().push(Piece {
            child: child.clone(),
            place,
            radius,
            progress: Rc::new(Cell::new(progress)),
            animation: None,
            open: progress > 0.,
            from: Rect::new(0., 0., 0., 0.),
            to: Rect::new(0., 0., 0., 0.),
        });
        self.queue_resize();
    }

    pub fn remove(&self, child: &impl IsA<gtk::Widget>) {
        let child = child.as_ref();
        let mut pieces = self.imp().pieces.borrow_mut();
        if let Some(i) = pieces.iter().position(|p| &p.child == child) {
            let piece = pieces.remove(i);
            if let Some(animation) = piece.animation {
                animation.pause();
            }
            piece.child.unparent();
        }
        drop(pieces);
        self.resized();
    }

    pub fn is_open(&self, child: &impl IsA<gtk::Widget>) -> bool {
        let child = child.as_ref();
        self.imp()
            .pieces
            .borrow()
            .iter()
            .any(|p| &p.child == child && p.open)
    }

    /// Whether any piece besides the always-open ones is showing.
    pub fn is_showing(&self, child: &impl IsA<gtk::Widget>) -> bool {
        let child = child.as_ref();
        self.imp()
            .pieces
            .borrow()
            .iter()
            .any(|p| &p.child == child && p.progress.get() > 0.)
    }

    /// Grows the piece out of its source.
    pub fn open(&self, child: &impl IsA<gtk::Widget>) {
        self.animate(child.as_ref(), true, None::<fn()>);
    }

    /// Shrinks the piece back into its source, then calls `done`.
    pub fn close(&self, child: &impl IsA<gtk::Widget>, done: Option<impl FnOnce() + 'static>) {
        self.animate(child.as_ref(), false, done);
    }

    fn animate(&self, child: &gtk::Widget, open: bool, done: Option<impl FnOnce() + 'static>) {
        let mut pieces = self.imp().pieces.borrow_mut();
        let Some(piece) = pieces.iter_mut().find(|p| &p.child == child) else {
            return;
        };
        if piece.open == open {
            drop(pieces);
            if let Some(done) = done {
                done();
            }
            return;
        }
        piece.open = open;
        if let Some(animation) = piece.animation.take() {
            animation.pause();
        }

        let progress = piece.progress.clone();
        let stage = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |value| {
            progress.set(value);
            if let Some(stage) = stage.upgrade() {
                stage.queue_draw();
            }
        });
        let (to, duration, easing) = if open {
            (1., OPEN_MS, adw::Easing::EaseOutCubic)
        } else {
            (0., CLOSE_MS, adw::Easing::EaseInOutCubic)
        };
        let animation = adw::TimedAnimation::builder()
            .widget(self)
            .value_from(piece.progress.get())
            .value_to(to)
            .duration(duration)
            .easing(easing)
            .target(&target)
            .build();

        let done = RefCell::new(done);
        let stage = self.downgrade();
        animation.connect_done(move |_| {
            if let Some(stage) = stage.upgrade() {
                if !open {
                    stage.resized();
                }
            }
            if let Some(done) = done.take() {
                done();
            }
        });
        piece.animation = Some(animation.clone());
        drop(pieces);

        if open {
            self.resized();
        }
        animation.play();
    }

    /// The stage's size follows its open pieces: it grows before one opens and shrinks after one
    /// closes.
    fn resized(&self) {
        self.queue_resize();
        if let Some(f) = &*self.imp().on_resize.borrow() {
            f();
        }
    }

    /// The pieces' current shapes take the input; the rest of the surface lets it through.
    fn set_input(&self, shapes: &[Shape]) {
        let Some(surface) = self.native().and_then(|native| native.surface()) else {
            return;
        };
        let region = gtk::cairo::Region::create();
        for shape in shapes {
            let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(
                shape.x,
                shape.y,
                shape.width,
                shape.height,
            ));
        }
        surface.set_input_region(&region);
    }

    /// Where a widget inside the stage is, in the stage's coordinates.
    pub fn rect_of(&self, widget: &impl IsA<gtk::Widget>) -> Option<Rect> {
        let bounds = widget.compute_bounds(self)?;
        Some(Rect::new(
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height(),
        ))
    }
}

/// A piece's natural size.
pub fn natural_size(widget: &impl IsA<gtk::Widget>) -> (f32, f32) {
    let (_, width, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let (_, height, _, _) = widget.measure(gtk::Orientation::Vertical, width);
    (width as f32, height as f32)
}
