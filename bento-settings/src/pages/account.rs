use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gdk, gio, glib};

use crate::account::{self, Account};
use crate::model::Category;
use crate::page::PageContext;

/// Pictures are scaled to this before AccountsService copies them, since it refuses big files.
const PICTURE_SIZE: i32 = 512;

pub fn category() -> Category {
    Category::custom("account", "Account", build)
        .icon("ph-user-circle-symbolic")
        .sidebar_row(sidebar_row)
        .keywords(&["user", "profile"])
        .search_item(
            "account.picture",
            "Picture",
            &["avatar", "photo", "profile picture"],
        )
        .search_item("account.name", "Name", &["full name", "real name"])
}

fn show_picture(avatar: &adw::Avatar, account: &Account) {
    avatar.set_text(Some(&account.name));
    let texture = account
        .picture
        .as_ref()
        .and_then(|path| gdk::Texture::from_filename(path).ok());
    avatar.set_custom_image(texture.as_ref());
}

/// The account at the top of the sidebar, in place of a plain title and icon.
fn sidebar_row() -> gtk::Widget {
    let account = account::current();
    let avatar = adw::Avatar::new(40, None, true);
    show_picture(&avatar, &account);

    let name = gtk::Label::builder()
        .xalign(0.)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    let user = gtk::Label::builder()
        .label(&account.user)
        .xalign(0.)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .build();
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    text.append(&user);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("account");
    row.append(&avatar);
    row.append(&text);

    let show = move |account: &Account| {
        show_picture(&avatar, account);
        name.set_label(&account.name);
        // Without a full name, the name already is the username.
        user.set_visible(account.user != account.name);
    };
    show(&account);
    account::subscribe(show);
    row.upcast()
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let account = Rc::new(RefCell::new(account::current()));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);

    let avatar = adw::Avatar::new(128, None, true);
    let menu = gio::Menu::new();
    menu.append(Some("Choose Picture…"), Some("account.choose-picture"));
    menu.append(Some("Remove Picture"), Some("account.remove-picture"));
    let edit = gtk::MenuButton::builder()
        .icon_name("ph-pencil-simple-symbolic")
        .tooltip_text("Change Picture")
        .menu_model(&menu)
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .css_classes(["circular", "osd"])
        .build();
    let picture = gtk::Overlay::builder()
        .child(&avatar)
        .halign(gtk::Align::Center)
        .build();
    picture.add_overlay(&edit);
    ctx.anchor("account.picture", &picture);
    column.append(&picture);

    let group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder()
        .title("Name")
        .show_apply_button(true)
        .build();
    {
        let (account, ctx) = (account.clone(), ctx.clone());
        name.connect_apply(move |row| {
            let text = row.text().trim().to_owned();
            let ctx = ctx.clone();
            account::set_name(&account.borrow(), &text, move |result| {
                if let Err(err) = result {
                    ctx.toast(&format!("Couldn't change the name: {err}"));
                }
            });
        });
    }
    ctx.anchor("account.name", &name);
    group.add(&name);
    group.add(&info_row("Username", &account.borrow().user));
    if let Some(administrator) = account.borrow().administrator {
        let kind = if administrator {
            "Administrator"
        } else {
            "Standard"
        };
        group.add(&info_row("Account Type", kind));
    }
    column.append(&group);

    let choose = gio::SimpleAction::new("choose-picture", None);
    {
        let (account, ctx) = (account.clone(), ctx.clone());
        choose.connect_activate(move |_, _| choose_picture(&account, &ctx));
    }
    let remove = gio::SimpleAction::new("remove-picture", None);
    {
        let (account, ctx) = (account.clone(), ctx.clone());
        remove.connect_activate(move |_, _| {
            let ctx = ctx.clone();
            account::set_picture(&account.borrow(), None, move |result| {
                if let Err(err) = result {
                    ctx.toast(&format!("Couldn't remove the picture: {err}"));
                }
            });
        });
    }
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&choose);
    actions.add_action(&remove);
    column.insert_action_group("account", Some(&actions));

    let editable = account.borrow().can_edit();
    name.set_sensitive(editable);
    edit.set_sensitive(editable);

    let show = move |current: &Account| {
        *account.borrow_mut() = current.clone();
        show_picture(&avatar, current);
        // Don't replace what someone is typing.
        if !name.has_focus() {
            name.set_text(&current.name);
        }
        remove.set_enabled(current.picture.is_some());
    };
    show(&account::current());
    account::subscribe(show);

    column.upcast()
}

fn choose_picture(account: &Rc<RefCell<Account>>, ctx: &PageContext) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_pixbuf_formats();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Picture")
        .filters(&filters)
        .build();

    let (account, ctx) = (account.clone(), ctx.clone());
    let window = ctx.toasts.root().and_downcast::<gtk::Window>();
    dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |result| {
        let Some(path) = result.ok().and_then(|file| file.path()) else {
            return;
        };
        let scaled = glib::user_runtime_dir().join("bento-settings-picture.png");
        let saved = Pixbuf::from_file_at_scale(&path, PICTURE_SIZE, PICTURE_SIZE, true)
            .and_then(|pixbuf| pixbuf.savev(&scaled, "png", &[]));
        if let Err(err) = saved {
            ctx.toast(&format!("Couldn't open the picture: {}", err.message()));
            return;
        }
        let copy = scaled.clone();
        account::set_picture(&account.borrow(), Some(&scaled), move |result| {
            // AccountsService has its own copy by now.
            let _ = std::fs::remove_file(&copy);
            if let Err(err) = result {
                ctx.toast(&format!("Couldn't change the picture: {err}"));
            }
        });
    });
}

fn info_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let label = gtk::Label::builder()
        .label(value)
        .selectable(true)
        .valign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build();
    row.add_suffix(&label);
    row
}
