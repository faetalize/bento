//! The stage: a surface's content as islands of glass.
//!
//! An island is one rounded rectangle of glass that takes the shape of one of its faces: its
//! resting face, like the clock, or a panel it opens into, like the calendar. Showing a face
//! springs the island's shape from where it is to that face's place, and the faces' content fades
//! across as the shape travels: the old one early, the new one late, so it reads as one piece of
//! glass becoming something else. A face can be nothing, a point, for something that grows out
//! of nothing and shrinks back into it.
//!
//! Each frame the stage draws the islands' content, and gives bento the same shapes as glass and
//! as the input region. The surface keeps its size while islands change shape, so a morph never
//! waits on bento; bento shades the glass itself, so the content goes straight on it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{graphene, gsk};

use crate::spring::{Spring, RESPONSE};
use crate::wayland::{Glass, Shape};

/// A rectangle on the stage, in logical pixels.
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

    /// Nothing, at the middle of `self`, for what grows out of nothing there.
    pub fn middle(self) -> Self {
        Self::new(self.x + self.width / 2., self.y + self.height / 2., 0., 0.)
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    fn graphene(self) -> graphene::Rect {
        graphene::Rect::new(self.x, self.y, self.width, self.height)
    }
}

/// Where a face puts its island, for the stage's width.
pub type Place = Rc<dyn Fn(f32) -> Rect>;

/// One shape an island can take.
pub struct Face {
    /// What shows in it, or `None` for nothing.
    pub child: Option<gtk::Widget>,
    pub place: Place,
    pub radius: f32,
    /// Where the content sits in the island while the island isn't its size yet, from 0 at the
    /// left or top to 1 at the right or bottom: the side the island grows from.
    pub align: (f32, f32),
}

impl Face {
    pub fn new(child: &impl IsA<gtk::Widget>, radius: f32, place: Place) -> Self {
        Self {
            child: Some(child.clone().upcast()),
            place,
            radius,
            align: (0.5, 0.),
        }
    }

    /// Nothing, at the middle of where `place` puts it. Its corners are as round as can be, so
    /// what shrinks into it rounds off on the way, into a drop, and what grows out of it starts
    /// as one.
    pub fn nothing(place: Place) -> Self {
        Self {
            child: None,
            place: Rc::new(move |width| place(width).middle()),
            radius: NOTHING_RADIUS,
            align: (0.5, 0.5),
        }
    }

    pub fn align(mut self, x: f32, y: f32) -> Self {
        self.align = (x, y);
        self
    }
}

/// The corner radius nothing heads for: past half of anything that shrinks into it, so it's round
/// well before it's gone, but not so far past that it's round from the start.
const NOTHING_RADIUS: f32 = 300.;

/// An island, by the order it was added in.
pub type IslandId = usize;

/// An island's x, y, width, height and corner radius.
const X: usize = 0;
const Y: usize = 1;
const W: usize = 2;
const H: usize = 3;
const R: usize = 4;

struct Island {
    faces: Vec<Face>,
    shown: usize,
    /// The face it was added showing, where its first layout puts it, so that one shown before
    /// then still grows into place.
    first: usize,
    /// How close other glass comes for it to melt into it.
    melt: f32,
    springs: [Spring; 5],
    /// Whether it has been laid out yet: until then it's put straight at its face.
    placed: bool,
    /// Its rectangle when the face it's showing was picked, to tell how far along it is.
    start: [f64; 4],
    /// How opaque each face's content was when the face it's showing was picked.
    faded_from: Vec<f64>,
    /// Called once it has settled into the face it's showing.
    done: Option<Box<dyn FnOnce()>>,
    /// Where each face puts it, from the last layout.
    places: Vec<Rect>,
}

impl Island {
    fn rect(&self) -> Rect {
        let s = &self.springs;
        Rect::new(
            s[X].value as f32,
            s[Y].value as f32,
            s[W].value.max(0.) as f32,
            s[H].value.max(0.) as f32,
        )
    }

    /// Its corners' radius as it's drawn: never more than half its width or height.
    fn radius(&self) -> f32 {
        let rect = self.rect();
        (self.springs[R].value as f32)
            .min(rect.width / 2.)
            .min(rect.height / 2.)
            .max(0.)
    }

    /// How far along the island is from where it was to the face it's showing, 0 to 1.
    fn progress(&self) -> f64 {
        let distance = |from: &[f64; 4]| {
            (0..4)
                .map(|i| (from[i] - self.springs[i].target).abs())
                .fold(0., f64::max)
        };
        let whole = distance(&self.start);
        if whole < 0.5 {
            return 1.;
        }
        let now = [X, Y, W, H].map(|i| self.springs[i].value);
        (1. - distance(&now) / whole).clamp(0., 1.)
    }

    /// How opaque face `i`'s content is: the face it's leaving fades out over the first third of
    /// the way, and the one it's going to fades in over the last half.
    fn opacity(&self, i: usize) -> f64 {
        let p = self.progress();
        let from = self.faded_from[i];
        if i == self.shown {
            from + (1. - from) * smoothstep(0.35, 0.85, p)
        } else {
            from * (1. - smoothstep(0., 0.35, p))
        }
    }

    fn settled(&mut self) -> bool {
        self.springs.iter_mut().all(|spring| spring.settle(0.05))
    }

    /// Where face `face` puts it, as its springs' targets, from the last layout.
    fn target(&self, face: usize) -> [f64; 5] {
        let rect = self.places[face];
        let radius = self.faces[face].radius;
        [rect.x, rect.y, rect.width, rect.height, radius].map(f64::from)
    }

    /// Lays it out with its faces at `places`: it heads for the one it shows, from where it is,
    /// or from the face it was added showing the first time.
    fn place(&mut self, places: Vec<Rect>) {
        self.places = places;
        let (first, shown) = (self.target(self.first), self.target(self.shown));
        if !self.placed {
            self.placed = true;
            for (spring, value) in self.springs.iter_mut().zip(first) {
                spring.jump(value);
            }
            self.start = [X, Y, W, H].map(|i| self.springs[i].value);
            self.faded_from = (0..self.faces.len())
                .map(|i| if i == self.first { 1. } else { 0. })
                .collect();
        }
        for (spring, value) in self.springs.iter_mut().zip(shown) {
            spring.target = value;
        }
    }

    /// Sets off for face `face`, from where it is.
    fn pick(&mut self, face: usize) {
        if self.shown == face {
            return;
        }
        self.faded_from = (0..self.faces.len()).map(|i| self.opacity(i)).collect();
        self.start = [X, Y, W, H].map(|i| self.springs[i].value);
        self.shown = face;
        self.target_shown();
        // It heads there now, from where the last layout put the face: the stage's frame tick
        // comes before the next layout, and would take it as arrived at the old face.
        if self.placed {
            let target = self.target(face);
            for (spring, value) in self.springs.iter_mut().zip(target) {
                spring.target = value;
            }
        }
    }

    /// Only the face it shows takes the pointer: the others are laid out in their places too,
    /// drawn faded out or not at all, and would take clicks meant for it.
    fn target_shown(&self) {
        for (i, face) in self.faces.iter().enumerate() {
            if let Some(child) = &face.child {
                child.set_can_target(i == self.shown);
            }
        }
    }
}

fn smoothstep(from: f64, to: f64, x: f64) -> f64 {
    let t = ((x - from) / (to - from)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Stage {
        pub(super) islands: RefCell<Vec<Option<Island>>>,
        pub glass: RefCell<Option<Glass>>,
        /// Room kept below the islands, for what grows out of them.
        pub room_below: Cell<f32>,
        pub(super) tick: RefCell<Option<gtk::TickCallbackId>>,
        pub(super) last_frame: Cell<Option<i64>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Stage {
        const NAME: &'static str = "BentoShellStage";
        type Type = super::Stage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Stage {
        fn dispose(&self) {
            for island in self.islands.take().into_iter().flatten() {
                for face in island.faces {
                    if let Some(child) = face.child {
                        child.unparent();
                    }
                }
            }
        }
    }

    impl WidgetImpl for Stage {
        fn unmap(&self) {
            // A hidden surface draws no frames, so the tick would never run again, and the stage
            // would take it as still going.
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
            self.parent_unmap();
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                return (0, 0, -1, -1);
            }
            // As tall as any face reaches, open or not, so that opening one never has to wait for
            // the surface to grow.
            let width = if for_size >= 0 {
                for_size
            } else {
                self.obj().width()
            } as f32;
            let places: Vec<Place> = self
                .islands
                .borrow()
                .iter()
                .flatten()
                .flat_map(|island| island.faces.iter().map(|face| face.place.clone()))
                .collect();
            let height = places
                .iter()
                .map(|place| place(width).bottom())
                .fold(0f32, f32::max)
                + self.room_below.get();
            let height = height.ceil() as i32;
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let width = width as f32;
            // The places measure the faces, so they're asked with nothing borrowed.
            let places: Vec<Option<Vec<(Place, Option<gtk::Widget>)>>> = self
                .islands
                .borrow()
                .iter()
                .map(|island| {
                    island.as_ref().map(|island| {
                        island
                            .faces
                            .iter()
                            .map(|face| (face.place.clone(), face.child.clone()))
                            .collect()
                    })
                })
                .collect();
            let placed: Vec<Option<Vec<(Rect, Option<gtk::Widget>)>>> = places
                .into_iter()
                .map(|faces| {
                    faces.map(|faces| {
                        faces
                            .into_iter()
                            .map(|(place, child)| (place(width), child))
                            .collect()
                    })
                })
                .collect();

            let mut moving = false;
            {
                let mut islands = self.islands.borrow_mut();
                for (island, faces) in islands.iter_mut().zip(&placed) {
                    let (Some(island), Some(faces)) = (island, faces) else {
                        continue;
                    };
                    // Each face's content sits where the island has it once there, so that it
                    // takes clicks there; while the island moves, it's drawn where it is then.
                    for (rect, child) in faces {
                        if let Some(child) = child {
                            child.size_allocate(
                                &gtk::Allocation::new(
                                    rect.x.round() as i32,
                                    rect.y.round() as i32,
                                    rect.width.round() as i32,
                                    rect.height.round() as i32,
                                ),
                                -1,
                            );
                        }
                    }
                    island.place(faces.iter().map(|(rect, _)| *rect).collect());
                    moving |= !island.settled();
                }
            }
            if moving {
                self.obj().ensure_ticking();
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let mut shapes = Vec::new();

            for island in self.islands.borrow().iter().flatten() {
                let rect = island.rect();
                if rect.width < 0.5 || rect.height < 0.5 {
                    continue;
                }
                let radius = island.radius();
                shapes.push(Shape {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    radius,
                    melt: island.melt,
                });

                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect.graphene(), radius));
                for (i, face) in island.faces.iter().enumerate() {
                    let Some(child) = &face.child else {
                        continue;
                    };
                    let opacity = island.opacity(i);
                    if opacity <= 0.003 {
                        continue;
                    }
                    // Where the island holds the content while it isn't its size: on the side it
                    // grows from, a little smaller while it fades in.
                    let at = island.places[i];
                    let (w, h) = (at.width.round(), at.height.round());
                    let x = rect.x + (rect.width - w) * face.align.0;
                    let y = rect.y + (rect.height - h) * face.align.1;
                    let pivot = graphene::Point::new(x + w * face.align.0, y + h * face.align.1);
                    let scale = 0.94 + 0.06 * opacity as f32;

                    snapshot.save();
                    snapshot.translate(&pivot);
                    snapshot.scale(scale, scale);
                    snapshot.translate(&graphene::Point::new(-pivot.x(), -pivot.y()));
                    snapshot.translate(&graphene::Point::new(x - at.x.round(), y - at.y.round()));
                    snapshot.push_opacity(opacity);
                    obj.snapshot_child(child, snapshot);
                    snapshot.pop();
                    snapshot.restore();
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

    /// Keeps `height` free below the islands, for what grows out of them.
    pub fn set_room_below(&self, height: f32) {
        self.imp().room_below.set(height);
        self.queue_resize();
    }

    /// Adds an island showing face `shown`, melting into other glass that comes `melt` close.
    pub fn add_island(&self, faces: Vec<Face>, shown: usize, melt: f32) -> IslandId {
        for child in faces.iter().filter_map(|face| face.child.as_ref()) {
            child.set_parent(self);
        }
        let count = faces.len();
        let mut islands = self.imp().islands.borrow_mut();
        let island = Island {
            faces,
            shown,
            first: shown,
            melt,
            springs: [Spring::new(0.); 5],
            placed: false,
            start: [0.; 4],
            faded_from: (0..count)
                .map(|i| if i == shown { 1. } else { 0. })
                .collect(),
            done: None,
            places: vec![Rect::new(0., 0., 0., 0.); count],
        };
        island.target_shown();
        islands.push(Some(island));
        let id = islands.len() - 1;
        drop(islands);
        self.queue_resize();
        id
    }

    /// Takes an island away at once.
    pub fn remove(&self, id: IslandId) {
        let island = self
            .imp()
            .islands
            .borrow_mut()
            .get_mut(id)
            .and_then(Option::take);
        if let Some(island) = island {
            for child in island.faces.into_iter().filter_map(|face| face.child) {
                child.unparent();
            }
        }
        self.queue_resize();
    }

    /// Which face an island shows, or is going to.
    pub fn shown(&self, id: IslandId) -> Option<usize> {
        self.imp()
            .islands
            .borrow()
            .get(id)
            .and_then(Option::as_ref)
            .map(|island| island.shown)
    }

    /// Morphs an island into face `face`.
    pub fn show(&self, id: IslandId, face: usize) {
        self.show_then(id, face, None::<fn()>);
    }

    /// Morphs an island into face `face`, then calls `done` once it has settled there, unless it's
    /// sent elsewhere first.
    pub fn show_then(&self, id: IslandId, face: usize, done: Option<impl FnOnce() + 'static>) {
        {
            let mut islands = self.imp().islands.borrow_mut();
            let Some(Some(island)) = islands.get_mut(id) else {
                return;
            };
            island.pick(face);
            island.done = done.map(|done| Box::new(done) as Box<dyn FnOnce()>);
        }
        // Its new face's place is set when it's laid out.
        self.queue_resize();
        self.ensure_ticking();
    }

    /// Lays the islands out again, as when what decides their places changed.
    pub fn relayout(&self) {
        self.queue_resize();
    }

    /// Moves the islands on each frame until they've settled.
    fn ensure_ticking(&self) {
        let imp = self.imp();
        if imp.tick.borrow().is_some() {
            return;
        }
        imp.last_frame.set(None);
        let id = self.add_tick_callback(|stage, clock| {
            let imp = stage.imp();
            let now = clock.frame_time();
            let dt = imp
                .last_frame
                .replace(Some(now))
                .map_or(1. / 60., |last| ((now - last) as f64 / 1e6).clamp(0., 0.05));

            let mut done = Vec::new();
            let mut moving = false;
            for island in imp.islands.borrow_mut().iter_mut().flatten() {
                for spring in &mut island.springs {
                    spring.step(dt, RESPONSE);
                }
                if island.settled() {
                    island.start = [X, Y, W, H].map(|i| island.springs[i].value);
                    island.faded_from = (0..island.faces.len())
                        .map(|i| if i == island.shown { 1. } else { 0. })
                        .collect();
                    done.extend(island.done.take());
                } else {
                    moving = true;
                }
            }
            stage.queue_draw();
            for done in done {
                done();
            }
            if moving {
                glib::ControlFlow::Continue
            } else {
                imp.tick.take();
                glib::ControlFlow::Break
            }
        });
        imp.tick.replace(Some(id));
    }

    /// The islands' current shapes take the input; the rest of the surface lets it through.
    fn set_input(&self, shapes: &[Shape]) {
        let Some(surface) = self.native().and_then(|native| native.surface()) else {
            return;
        };
        let region = gtk::cairo::Region::create();
        for shape in shapes {
            let (x0, y0) = (shape.x.floor() as i32, shape.y.floor() as i32);
            let (x1, y1) = (
                (shape.x + shape.width).ceil() as i32,
                (shape.y + shape.height).ceil() as i32,
            );
            let _ =
                region.union_rectangle(&gtk::cairo::RectangleInt::new(x0, y0, x1 - x0, y1 - y0));
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

/// A widget's natural size.
pub fn natural_size(widget: &impl IsA<gtk::Widget>) -> (f32, f32) {
    let (_, width, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let (_, height, _, _) = widget.measure(gtk::Orientation::Vertical, width);
    (width as f32, height as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn island(faces: &[Rect]) -> Island {
        let count = faces.len();
        let faces: Vec<Face> = faces
            .iter()
            .map(|rect| {
                let rect = *rect;
                Face {
                    child: None,
                    place: Rc::new(move |_| rect),
                    radius: 0.,
                    align: (0.5, 0.5),
                }
            })
            .collect();
        Island {
            faces,
            shown: 0,
            first: 0,
            melt: 0.,
            springs: [Spring::new(0.); 5],
            placed: false,
            start: [0.; 4],
            faded_from: (0..count).map(|i| if i == 0 { 1. } else { 0. }).collect(),
            done: None,
            places: vec![Rect::new(0., 0., 0., 0.); count],
        }
    }

    #[test]
    fn a_face_shown_before_the_first_layout_still_grows_into_place() {
        // Like the launcher: added as nothing, and shown before GTK has laid it out.
        let nothing = Rect::new(100., 50., 0., 0.);
        let shown = Rect::new(0., 0., 200., 100.);
        let mut island = island(&[nothing, shown]);
        island.pick(1);
        island.place(vec![nothing, shown]);

        assert_eq!(island.rect(), nothing);
        assert_eq!(island.springs[W].target, 200.);
        assert_eq!(island.opacity(1), 0.);
    }

    #[test]
    fn a_picked_face_is_where_it_heads_before_the_next_layout() {
        // Like the launcher closing: it's at its face, and is sent back to nothing, which the
        // stage's frame tick sees before GTK lays it out again.
        let nothing = Rect::new(100., 50., 0., 0.);
        let shown = Rect::new(0., 0., 200., 100.);
        let mut island = island(&[nothing, shown]);
        island.pick(1);
        island.place(vec![nothing, shown]);
        for spring in &mut island.springs {
            spring.jump(spring.target);
        }

        island.pick(0);
        assert_eq!(island.springs[W].target, 0.);
        assert!(!island.settled(), "it has to shrink first");
    }

    #[test]
    fn it_shrinks_into_nothing_as_a_drop() {
        let shown = Rect::new(0., 0., 200., 100.);
        let mut island = island(&[shown, shown]);
        island.faces[0] = Face::nothing(Rc::new(move |_| shown));
        island.faces[1].radius = 24.;
        island.pick(1);
        island.place(vec![shown.middle(), shown]);
        for spring in &mut island.springs {
            spring.jump(spring.target);
        }

        island.pick(0);
        while island.rect().width > 20. {
            for spring in &mut island.springs {
                spring.step(1. / 60., RESPONSE);
            }
        }
        let rect = island.rect();
        let round = rect.width.min(rect.height) / 2.;
        assert!(
            island.radius() >= round - 0.01,
            "{rect:?} with corners of {}",
            island.radius()
        );
    }
}
