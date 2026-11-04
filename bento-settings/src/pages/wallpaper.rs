//! The wallpaper: the current one shown large, and the pictures in its folder to pick from.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gdk, gio, glib};

use crate::kdl::Value;
use crate::page::{open_file, PageContext};
use crate::store::tilde;

const KEY: &str = "layout.background-image";
const PATH: &[&str] = &["layout", "background-image"];

/// What bento can decode.
const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Pixel height thumbnails and the large preview are loaded at.
const THUMBNAIL_HEIGHT: i32 = 180;
const PREVIEW_HEIGHT: i32 = 720;

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Wallpaper").build();
    ctx.anchor("appearance.wallpaper", &group);

    let folder = folder(current(ctx).as_deref());

    let open_folder = gtk::Button::builder()
        .icon_name("ph-folder-open-symbolic")
        .tooltip_text("Open Folder")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let folder = folder.clone();
        open_folder.connect_clicked(move |button| open_file(button, &folder));
    }
    group.set_header_suffix(Some(&open_folder));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let preview = Preview::new(ctx);
    content.append(&preview.frame);

    let grid = gtk::FlowBox::builder()
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .column_spacing(6)
        .row_spacing(6)
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .css_classes(["wallpapers"])
        .build();
    let tiles: Rc<RefCell<Vec<(PathBuf, gtk::FlowBoxChild)>>> = Rc::default();
    for path in pictures(&folder) {
        let child = tile(&path);
        grid.append(&child);
        tiles.borrow_mut().push((path, child));
    }
    let add = add_tile();
    grid.append(&add);
    content.append(&grid);
    group.add(&content);

    {
        let (ctx, tiles, add) = (ctx.clone(), tiles.clone(), add.clone());
        grid.connect_child_activated(move |grid, child| {
            if *child == add {
                choose(grid, &ctx);
                return;
            }
            let tiles = tiles.borrow();
            if let Some((path, _)) = tiles.iter().find(|(_, tile)| tile == child) {
                set(&ctx, Some(path));
                mark(&tiles, Some(path));
            }
        });
    }

    let show = {
        let ctx = ctx.clone();
        move || {
            // A pick still on its way to the config shouldn't flicker back.
            if ctx.store.is_pending(KEY) {
                return;
            }
            let current = current(&ctx);
            preview.show(current.as_deref());
            mark(&tiles.borrow(), current.as_deref());
        }
    };
    show();
    ctx.store.subscribe(show);

    group.upcast()
}

/// The current wallpaper, with `~` expanded.
fn current(ctx: &PageContext) -> Option<PathBuf> {
    let path = ctx.store.config().layout.background_image.clone()?;
    Some(match path.strip_prefix("~/") {
        Some(rest) => glib::home_dir().join(rest),
        None => PathBuf::from(path),
    })
}

/// Where to find pictures: next to the current wallpaper, or the usual wallpaper folders.
fn folder(current: Option<&Path>) -> PathBuf {
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)
        .unwrap_or_else(|| glib::home_dir().join("Pictures"));
    let wallpapers = pictures.join("Wallpapers");
    current
        .and_then(Path::parent)
        .map(Path::to_owned)
        .filter(|dir| dir.is_dir())
        .or_else(|| wallpapers.is_dir().then_some(wallpapers))
        .unwrap_or(pictures)
}

fn pictures(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        })
        .collect();
    paths.sort();
    paths
}

fn set(ctx: &PageContext, path: Option<&Path>) {
    let value = path.map(tilde);
    ctx.store.edit(KEY, move |doc| match value {
        Some(value) => doc.set(PATH, vec![Value::Str(value)]),
        None => {
            doc.remove(PATH);
        }
    });
}

fn mark(tiles: &[(PathBuf, gtk::FlowBoxChild)], current: Option<&Path>) {
    for (path, tile) in tiles {
        if Some(path.as_path()) == current {
            tile.add_css_class("current");
        } else {
            tile.remove_css_class("current");
        }
    }
}

fn tile(path: &Path) -> gtk::FlowBoxChild {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["wallpaper-thumbnail"])
        .build();
    let frame = gtk::AspectFrame::new(0.5, 0.5, 16. / 10., false);
    frame.set_child(Some(&picture));
    load(path, THUMBNAIL_HEIGHT, &picture);

    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned());
    gtk::FlowBoxChild::builder()
        .child(&frame)
        .tooltip_text(name.unwrap_or_default())
        .build()
}

fn add_tile() -> gtk::FlowBoxChild {
    let icon = gtk::Image::builder()
        .icon_name("ph-plus-symbolic")
        .pixel_size(24)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let inner = gtk::Box::builder().css_classes(["wallpaper-add"]).build();
    inner.append(&icon);
    icon.set_hexpand(true);
    let frame = gtk::AspectFrame::new(0.5, 0.5, 16. / 10., false);
    frame.set_child(Some(&inner));
    gtk::FlowBoxChild::builder()
        .child(&frame)
        .tooltip_text("Add Picture…")
        .build()
}

fn choose(parent: &impl IsA<gtk::Widget>, ctx: &PageContext) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    for ext in EXTENSIONS {
        filter.add_suffix(ext);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Wallpaper")
        .filters(&filters)
        .build();
    let window = parent.root().and_downcast::<gtk::Window>();
    let ctx = ctx.clone();
    dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |result| {
        if let Some(path) = result.ok().and_then(|file| file.path()) {
            set(&ctx, Some(&path));
        }
    });
}

/// The current wallpaper, large, with its name and a button to remove it.
struct Preview {
    frame: gtk::AspectFrame,
    picture: gtk::Picture,
    empty: gtk::Box,
    name: gtk::Label,
    remove: gtk::Button,
    shown: RefCell<Option<PathBuf>>,
}

impl Preview {
    fn new(ctx: &PageContext) -> Rc<Self> {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .build();
        let empty = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .valign(gtk::Align::Center)
            .build();
        empty.append(
            &gtk::Image::builder()
                .icon_name("ph-image-symbolic")
                .pixel_size(48)
                .css_classes(["dim-label"])
                .build(),
        );
        empty.append(
            &gtk::Label::builder()
                .label("No Wallpaper")
                .css_classes(["dim-label", "title-4"])
                .build(),
        );
        let name = gtk::Label::builder()
            .xalign(0.)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .valign(gtk::Align::End)
            .css_classes(["wallpaper-name", "heading"])
            .build();
        let remove = gtk::Button::builder()
            .icon_name("ph-x-symbolic")
            .tooltip_text("Remove Wallpaper")
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(10)
            .margin_end(10)
            .css_classes(["circular", "osd"])
            .build();
        {
            let ctx = ctx.clone();
            remove.connect_clicked(move |_| set(&ctx, None));
        }

        let overlay = gtk::Overlay::builder()
            .child(&picture)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["wallpaper-preview"])
            .build();
        overlay.add_overlay(&empty);
        overlay.add_overlay(&name);
        overlay.add_overlay(&remove);
        let frame = gtk::AspectFrame::new(0.5, 0.5, 16. / 10., false);
        frame.set_child(Some(&overlay));

        Rc::new(Self {
            frame,
            picture,
            empty,
            name,
            remove,
            shown: RefCell::new(None),
        })
    }

    fn show(&self, path: Option<&Path>) {
        if self.shown.borrow().as_deref() == path {
            return;
        }
        self.shown.replace(path.map(Path::to_owned));
        self.empty.set_visible(path.is_none());
        self.name.set_visible(path.is_some());
        self.remove.set_visible(path.is_some());
        self.picture.set_paintable(None::<&gdk::Paintable>);
        if let Some(path) = path {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            self.name.set_label(&name.unwrap_or_default());
            load(path, PREVIEW_HEIGHT, &self.picture);
        }
    }
}

/// Loads a scaled-down copy of an image on a background thread, and shows it when it's ready.
fn load(path: &Path, height: i32, picture: &gtk::Picture) {
    let path = path.to_owned();
    let picture = picture.downgrade();
    glib::spawn_future_local(async move {
        let decoded = gio::spawn_blocking(move || {
            let pixbuf = Pixbuf::from_file_at_scale(&path, -1, height, true).ok()?;
            Some((
                pixbuf.width(),
                pixbuf.height(),
                pixbuf.rowstride() as usize,
                pixbuf.has_alpha(),
                pixbuf.read_pixel_bytes(),
            ))
        })
        .await
        .ok()
        .flatten();
        let (Some(picture), Some((width, height, stride, alpha, bytes))) =
            (picture.upgrade(), decoded)
        else {
            return;
        };
        let format = if alpha {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        let texture = gdk::MemoryTexture::new(width, height, format, &bytes, stride);
        picture.set_paintable(Some(&texture));
    });
}
