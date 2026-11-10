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
        .search_item("account.email", "Email", &["mail", "address"])
        .search_item(
            "account.password",
            "Password",
            &["change password", "passwd"],
        )
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
        // The email if there is one, else the username, unless the name already is it.
        let subtitle = if account.email.is_empty() {
            &account.user
        } else {
            &account.email
        };
        user.set_label(subtitle);
        user.set_visible(*subtitle != account.name);
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

    let email = adw::EntryRow::builder()
        .title("Email")
        .input_purpose(gtk::InputPurpose::Email)
        .show_apply_button(true)
        .build();
    {
        let (account, ctx) = (account.clone(), ctx.clone());
        email.connect_apply(move |row| {
            let text = row.text().trim().to_owned();
            let ctx = ctx.clone();
            account::set_email(&account.borrow(), &text, move |result| {
                if let Err(err) = result {
                    ctx.toast(&format!("Couldn't change the email: {err}"));
                }
            });
        });
    }
    ctx.anchor("account.email", &email);
    group.add(&email);
    group.add(&crate::page::info_row("Username", &account.borrow().user));
    if let Some(administrator) = account.borrow().administrator {
        let kind = if administrator {
            "Administrator"
        } else {
            "Standard"
        };
        group.add(&crate::page::info_row("Account Type", kind));
    }
    column.append(&group);

    let security = adw::PreferencesGroup::new();
    let password = adw::ActionRow::builder()
        .title("Password")
        .activatable(true)
        .build();
    password.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let ctx = ctx.clone();
        password.connect_activated(move |row| password_dialog(row, &ctx));
    }
    ctx.anchor("account.password", &password);
    security.add(&password);
    if let Some(last_login) = account.borrow().last_login {
        security.add(&crate::page::info_row(
            "Last Login",
            &format_time(last_login),
        ));
    }
    column.append(&security);

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
    email.set_sensitive(editable);
    edit.set_sensitive(editable);

    let show = move |current: &Account| {
        *account.borrow_mut() = current.clone();
        show_picture(&avatar, current);
        // Don't replace what someone is typing.
        if !crate::page::typing_in(&name) {
            name.set_text(&current.name);
        }
        if !crate::page::typing_in(&email) {
            email.set_text(&current.email);
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

fn format_time(seconds: i64) -> String {
    glib::DateTime::from_unix_local(seconds)
        .and_then(|time| time.format("%e %B %Y, %H:%M"))
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// Asks for the current password and the new one twice, like GNOME Settings does.
fn password_dialog(parent: &impl IsA<gtk::Widget>, ctx: &PageContext) {
    let current = adw::PasswordEntryRow::builder()
        .title("Current Password")
        .build();
    let new = adw::PasswordEntryRow::builder()
        .title("New Password")
        .build();
    let confirm = adw::PasswordEntryRow::builder()
        .title("Confirm New Password")
        .build();
    let group = adw::PreferencesGroup::new();
    group.add(&current);
    group.add(&new);
    group.add(&confirm);

    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.)
        .visible(false)
        .css_classes(["error"])
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.add_css_class("dialog-content");
    column.append(&group);
    column.append(&error);

    let change = gtk::Button::builder()
        .label("Change")
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&change);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&column));
    let dialog = adw::Dialog::builder()
        .title("Change Password")
        .content_width(420)
        .child(&toolbar)
        .default_widget(&change)
        .build();

    let validate = {
        let (current, new, confirm, change, error) = (
            current.clone(),
            new.clone(),
            confirm.clone(),
            change.clone(),
            error.clone(),
        );
        move || {
            let mismatch = !confirm.text().is_empty() && new.text() != confirm.text();
            error.set_label("The new passwords don't match");
            error.set_visible(mismatch);
            change.set_sensitive(
                !current.text().is_empty()
                    && !new.text().is_empty()
                    && !mismatch
                    && !confirm.text().is_empty(),
            );
        }
    };
    for row in [&current, &new, &confirm] {
        let validate = validate.clone();
        row.connect_changed(move |_| validate());
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        let ctx = ctx.clone();
        change.connect_clicked(move |button| {
            button.set_sensitive(false);
            let (current_text, new_text) = (current.text().to_string(), new.text().to_string());
            let (dialog, ctx, error, button) =
                (dialog.clone(), ctx.clone(), error.clone(), button.clone());
            glib::spawn_future_local(async move {
                match crate::password::change(&current_text, &new_text).await {
                    Ok(()) => {
                        dialog.close();
                        ctx.toast("Password changed");
                    }
                    Err(err) => {
                        error.set_label(&err);
                        error.set_visible(true);
                        button.set_sensitive(true);
                    }
                }
            });
        });
    }
    dialog.present(Some(parent));
}
