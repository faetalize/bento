//! Apps' status icons, through the StatusNotifierItem protocol and its dbusmenu menus.
//!
//! The shell is a StatusNotifierHost, and the StatusNotifierWatcher too when no other program is.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gio::prelude::*;
use gio::{
    BusNameOwnerFlags, BusType, DBusCallFlags, DBusConnection, DBusMethodInvocation, DBusNodeInfo,
    DBusSignalFlags, OwnerId, RegistrationId, SignalSubscription,
};
use glib::{Variant, VariantDict};
use gtk::gdk;

use super::dbus;

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// The prefix of the action group that [`Tray::menu`] makes.
pub const ACTION_PREFIX: &str = "tray";

const WATCHER_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem">
      <arg type="s" name="service" direction="in"/>
    </method>
    <method name="RegisterStatusNotifierHost">
      <arg type="s" name="service" direction="in"/>
    </method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered">
      <arg type="s" name="service"/>
    </signal>
    <signal name="StatusNotifierItemUnregistered">
      <arg type="s" name="service"/>
    </signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
</node>
"#;

/// Whether an item wants to be seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// The item has nothing to say; it isn't shown.
    Passive,
    #[default]
    Active,
    NeedsAttention,
}

#[derive(Debug, Clone)]
pub enum TrayIcon {
    /// An icon from the icon theme, or from `theme_path` when the app ships its own.
    Name {
        name: String,
        theme_path: Option<String>,
    },
    Texture(gdk::Texture),
}

/// An app's status icon.
#[derive(Debug, Clone)]
pub struct TrayItem {
    /// Where the item is on the bus, which [`Tray`]'s methods take.
    pub id: String,
    pub title: String,
    pub status: Status,
    /// The attention icon while the item needs attention, when it has one.
    pub icon: Option<TrayIcon>,
    /// Whether the item only has a menu, so clicking it shows the menu rather than activating it.
    pub item_is_menu: bool,
}

/// The shell's tray: the status icons of the apps that have one, in the order they appeared.
pub struct Tray {
    _watcher: Rc<Watcher>,
    host_name: String,
    host_owner: Cell<Option<OwnerId>>,
    host_owned: Cell<bool>,
    /// The watcher, whoever it is.
    watcher_watch: RefCell<Option<dbus::NameWatch>>,
    bus: RefCell<Option<DBusConnection>>,
    watcher_signals: RefCell<Vec<SignalSubscription>>,
    items: RefCell<Vec<Item>>,
    on_change: Listener,
}

type Listener = Box<dyn Fn(&[TrayItem])>;

/// What the tray keeps about an item.
struct Item {
    bus_name: String,
    path: String,
    /// `None` until its properties have been read.
    shown: Option<TrayItem>,
    menu: Option<String>,
    generation: u64,
    _signals: SignalSubscription,
}

impl Tray {
    pub fn start(on_change: impl Fn(&[TrayItem]) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            _watcher: Watcher::start(),
            host_name: format!("org.kde.StatusNotifierHost-{}", std::process::id()),
            host_owner: Cell::default(),
            host_owned: Cell::new(false),
            watcher_watch: RefCell::default(),
            bus: RefCell::default(),
            watcher_signals: RefCell::default(),
            items: RefCell::default(),
            on_change: Box::new(on_change),
        });

        let acquired = Rc::downgrade(&this);
        let owner = gio::bus_own_name(
            BusType::Session,
            &this.host_name,
            BusNameOwnerFlags::NONE,
            |_, _| (),
            move |_, _| {
                if let Some(this) = acquired.upgrade() {
                    this.host_owned.set(true);
                    this.register_host();
                }
            },
            |_, name| eprintln!("Couldn't own {name}, so the tray stays empty"),
        );
        this.host_owner.set(Some(owner));

        let appeared = Rc::downgrade(&this);
        let vanished = Rc::downgrade(&this);
        let watch = dbus::watch_name(
            BusType::Session,
            WATCHER_NAME,
            move |bus| {
                if let Some(this) = appeared.upgrade() {
                    this.watcher_appeared(bus);
                }
            },
            move || {
                if let Some(this) = vanished.upgrade() {
                    this.watcher_vanished();
                }
            },
        );
        *this.watcher_watch.borrow_mut() = Some(watch);
        this
    }

    /// Activates an item, like clicking it. `x` and `y` are where, on the screen.
    pub fn activate(&self, id: &str, x: i32, y: i32) {
        self.call_item(id, "Activate", (x, y).to_variant());
    }

    /// Does an item's secondary action, like middle-clicking it.
    pub fn secondary_activate(&self, id: &str, x: i32, y: i32) {
        self.call_item(id, "SecondaryActivate", (x, y).to_variant());
    }

    /// Asks an item to show its own menu, for items without a dbusmenu.
    pub fn context_menu(&self, id: &str, x: i32, y: i32) {
        self.call_item(id, "ContextMenu", (x, y).to_variant());
    }

    /// Builds an item's menu, with its actions in a group for the [`ACTION_PREFIX`] prefix, and
    /// hands it to `ready`. It's `None` when the item has no menu or the menu can't be read.
    pub fn menu(
        &self,
        id: &str,
        ready: impl FnOnce(Option<(gio::MenuModel, gio::SimpleActionGroup)>) + 'static,
    ) {
        let bus = self.bus.borrow().clone();
        let target = self.items.borrow().iter().find_map(|item| {
            let menu = item.menu.clone()?;
            (item_id(item) == id).then(|| (item.bus_name.clone(), menu))
        });
        let (Some(bus), Some((bus_name, path))) = (bus, target) else {
            ready(None);
            return;
        };

        glib::spawn_future_local(async move {
            // Lets the app fill the menu in before it's read.
            let _ = bus
                .call_future(
                    Some(&bus_name),
                    &path,
                    MENU,
                    "AboutToShow",
                    Some(&(0i32,).to_variant()),
                    None,
                    DBusCallFlags::NONE,
                    -1,
                )
                .await;
            let layout = bus
                .call_future(
                    Some(&bus_name),
                    &path,
                    MENU,
                    "GetLayout",
                    Some(&(0i32, -1i32, Vec::<String>::new()).to_variant()),
                    Some(glib::VariantTy::new("(u(ia{sv}av))").unwrap()),
                    DBusCallFlags::NONE,
                    -1,
                )
                .await;
            let root = match layout {
                Ok(reply) => MenuNode::parse(&reply.child_value(1)),
                Err(err) => {
                    eprintln!("Couldn't read the menu of {bus_name}: {err}");
                    None
                }
            };
            ready(root.map(|root| build_menu(&root, &bus, &bus_name, &path)));
        });
    }

    fn call_item(&self, id: &str, method: &'static str, parameters: Variant) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let target = self
            .items
            .borrow()
            .iter()
            .find(|item| item_id(item) == id)
            .map(|item| (item.bus_name.clone(), item.path.clone()));
        let Some((bus_name, path)) = target else {
            return;
        };
        glib::spawn_future_local(async move {
            let reply = bus
                .call_future(
                    Some(&bus_name),
                    &path,
                    ITEM,
                    method,
                    Some(&parameters),
                    None,
                    DBusCallFlags::NONE,
                    -1,
                )
                .await;
            if let Err(err) = reply {
                eprintln!("Couldn't {method} {bus_name}: {err}");
            }
        });
    }

    fn watcher_appeared(self: &Rc<Self>, bus: DBusConnection) {
        let signal = |member: &'static str, f: fn(&Rc<Self>, String)| {
            let weak = Rc::downgrade(self);
            bus.subscribe_to_signal(
                Some(WATCHER_NAME),
                Some(WATCHER),
                Some(member),
                Some(WATCHER_PATH),
                None,
                DBusSignalFlags::NONE,
                move |signal| {
                    let address = signal.parameters.get::<(String,)>();
                    if let (Some(this), Some((address,))) = (weak.upgrade(), address) {
                        f(&this, address);
                    }
                },
            )
        };
        *self.watcher_signals.borrow_mut() = vec![
            signal("StatusNotifierItemRegistered", Self::add_item),
            signal("StatusNotifierItemUnregistered", Self::remove_item),
        ];
        *self.bus.borrow_mut() = Some(bus.clone());
        self.register_host();

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let items = dbus::property(
                &bus,
                WATCHER_NAME,
                WATCHER_PATH,
                WATCHER,
                "RegisteredStatusNotifierItems",
            )
            .await;
            let Some(this) = weak.upgrade() else { return };
            match items.map(|items| items.get::<Vec<String>>()) {
                Ok(Some(items)) if items.is_empty() => this.notify(),
                Ok(Some(items)) => {
                    for address in items {
                        this.add_item(address);
                    }
                }
                Ok(None) => eprintln!("The status notifier watcher listed its items wrong"),
                Err(err) => eprintln!("Couldn't list the status icons: {err}"),
            }
        });
    }

    fn watcher_vanished(&self) {
        self.watcher_signals.borrow_mut().clear();
        self.bus.take();
        self.items.borrow_mut().clear();
        self.notify();
    }

    /// Tells the watcher about the host, once the host has its name and there is a watcher.
    fn register_host(&self) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        if !self.host_owned.get() {
            return;
        }
        let host_name = self.host_name.clone();
        glib::spawn_future_local(async move {
            let reply = bus
                .call_future(
                    Some(WATCHER_NAME),
                    WATCHER_PATH,
                    WATCHER,
                    "RegisterStatusNotifierHost",
                    Some(&(host_name,).to_variant()),
                    None,
                    DBusCallFlags::NONE,
                    -1,
                )
                .await;
            if let Err(err) = reply {
                eprintln!("Couldn't register the tray with the status notifier watcher: {err}");
            }
        });
    }

    fn add_item(self: &Rc<Self>, address: String) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let (bus_name, path) = split_address(&address);
        if self
            .items
            .borrow()
            .iter()
            .any(|item| item.bus_name == bus_name && item.path == path)
        {
            return;
        }

        let weak = Rc::downgrade(self);
        let (signal_bus, signal_path) = (bus_name.clone(), path.clone());
        let signals = bus.subscribe_to_signal(
            Some(&bus_name),
            Some(ITEM),
            None,
            Some(&path),
            None,
            DBusSignalFlags::NONE,
            move |signal| {
                if signal.signal_name.starts_with("New") {
                    if let Some(this) = weak.upgrade() {
                        this.refresh_item(&signal_bus, &signal_path);
                    }
                }
            },
        );
        self.items.borrow_mut().push(Item {
            bus_name: bus_name.clone(),
            path: path.clone(),
            shown: None,
            menu: None,
            generation: 0,
            _signals: signals,
        });
        self.refresh_item(&bus_name, &path);
    }

    fn remove_item(self: &Rc<Self>, address: String) {
        let (bus_name, path) = split_address(&address);
        let mut items = self.items.borrow_mut();
        let len = items.len();
        items.retain(|item| !(item.bus_name == bus_name && item.path == path));
        let changed = items.len() != len;
        drop(items);
        if changed {
            self.notify();
        }
    }

    /// Reads an item's properties again.
    fn refresh_item(self: &Rc<Self>, bus_name: &str, path: &str) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let generation = {
            let mut items = self.items.borrow_mut();
            let Some(item) = items
                .iter_mut()
                .find(|i| i.bus_name == bus_name && i.path == path)
            else {
                return;
            };
            item.generation += 1;
            item.generation
        };

        let weak = Rc::downgrade(self);
        let (bus_name, path) = (bus_name.to_owned(), path.to_owned());
        glib::spawn_future_local(async move {
            let properties = dbus::properties(&bus, &bus_name, &path, ITEM).await;
            let Some(this) = weak.upgrade() else { return };
            let mut items = this.items.borrow_mut();
            let Some(index) = items
                .iter()
                .position(|i| i.bus_name == bus_name && i.path == path)
            else {
                return;
            };
            if items[index].generation != generation {
                return;
            }
            match properties {
                Ok(properties) => {
                    let properties = Properties::parse(&properties);
                    let item = &mut items[index];
                    item.menu = properties.menu.clone();
                    item.shown = Some(properties.into_item(format!("{bus_name}{path}")));
                }
                Err(err) => {
                    eprintln!("Couldn't read the status icon of {bus_name}: {err}");
                    items.remove(index);
                }
            }
            drop(items);
            this.notify();
        });
    }

    /// The items to show, in the order they appeared.
    pub fn items(&self) -> Vec<TrayItem> {
        self.items
            .borrow()
            .iter()
            .filter_map(|item| item.shown.clone())
            .filter(|item| item.status != Status::Passive)
            .collect()
    }

    fn notify(&self) {
        (self.on_change)(&self.items());
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Some(owner) = self.host_owner.take() {
            gio::bus_unown_name(owner);
        }
    }
}

fn item_id(item: &Item) -> String {
    format!("{}{}", item.bus_name, item.path)
}

/// Splits an item's address, its bus name followed by its object path, which defaults to
/// `/StatusNotifierItem`.
fn split_address(address: &str) -> (String, String) {
    match address.find('/') {
        Some(slash) => (address[..slash].to_owned(), address[slash..].to_owned()),
        None => (address.to_owned(), String::from(ITEM_PATH)),
    }
}

/// The address the watcher lists for an item that registered with `service`, which is its bus
/// name, its object path on the sender's connection, or both.
fn item_address(service: &str, sender: &str) -> String {
    if service.starts_with('/') {
        format!("{sender}{service}")
    } else if service.contains('/') {
        service.to_owned()
    } else {
        format!("{service}{ITEM_PATH}")
    }
}

/// An item's properties, as the tray uses them.
#[derive(Debug, Default, PartialEq)]
struct Properties {
    id: String,
    title: String,
    status: Status,
    icon_name: String,
    attention_icon_name: String,
    theme_path: Option<String>,
    pixmap: Option<Pixmap>,
    attention_pixmap: Option<Pixmap>,
    menu: Option<String>,
    item_is_menu: bool,
}

impl Properties {
    fn parse(properties: &VariantDict) -> Self {
        let string = |key| dbus::lookup::<String>(properties, key).unwrap_or_default();
        let pixmap = |key| {
            properties
                .lookup_value(key, None)
                .and_then(|value| Pixmap::largest(&value))
        };
        Self {
            id: string("Id"),
            title: string("Title"),
            status: match string("Status").as_str() {
                "Passive" => Status::Passive,
                "NeedsAttention" => Status::NeedsAttention,
                _ => Status::Active,
            },
            icon_name: string("IconName"),
            attention_icon_name: string("AttentionIconName"),
            theme_path: Some(string("IconThemePath")).filter(|path| !path.is_empty()),
            pixmap: pixmap("IconPixmap"),
            attention_pixmap: pixmap("AttentionIconPixmap"),
            menu: Some(string("Menu")).filter(|path| !path.is_empty() && path != "/"),
            item_is_menu: dbus::lookup::<bool>(properties, "ItemIsMenu").unwrap_or(false),
        }
    }

    fn into_item(self, id: String) -> TrayItem {
        let attention = self.status == Status::NeedsAttention
            && (!self.attention_icon_name.is_empty() || self.attention_pixmap.is_some());
        let (name, pixmap) = if attention {
            (self.attention_icon_name, self.attention_pixmap)
        } else {
            (self.icon_name, self.pixmap)
        };
        let icon = if name.starts_with('/') {
            match gdk::Texture::from_filename(&name) {
                Ok(texture) => Some(TrayIcon::Texture(texture)),
                Err(_) => pixmap.map(|pixmap| TrayIcon::Texture(pixmap.texture())),
            }
        } else if !name.is_empty() {
            Some(TrayIcon::Name {
                name,
                theme_path: self.theme_path,
            })
        } else {
            pixmap.map(|pixmap| TrayIcon::Texture(pixmap.texture()))
        };
        TrayItem {
            id,
            title: if self.title.is_empty() {
                self.id
            } else {
                self.title
            },
            status: self.status,
            icon,
            item_is_menu: self.item_is_menu,
        }
    }
}

/// An icon sent as pixels: ARGB32 in network byte order, so A, R, G, B bytes.
#[derive(Debug, PartialEq)]
struct Pixmap {
    width: i32,
    height: i32,
    data: Vec<u8>,
}

impl Pixmap {
    /// The largest of the sizes in an `a(iiay)`.
    fn largest(value: &Variant) -> Option<Self> {
        let pixmaps = value.get::<Vec<(i32, i32, Vec<u8>)>>()?;
        pixmaps
            .into_iter()
            .filter(|(width, height, data)| {
                *width > 0 && *height > 0 && data.len() >= *width as usize * *height as usize * 4
            })
            .max_by_key(|(width, height, _)| i64::from(*width) * i64::from(*height))
            .map(|(width, height, data)| Self {
                width,
                height,
                data,
            })
    }

    fn texture(self) -> gdk::Texture {
        let stride = self.width as usize * 4;
        let bytes = glib::Bytes::from_owned(self.data);
        let texture = gdk::MemoryTexture::new(
            self.width,
            self.height,
            gdk::MemoryFormat::A8r8g8b8,
            &bytes,
            stride,
        );
        texture.upcast()
    }
}

/// A dbusmenu item, as `GetLayout` describes it.
#[derive(Debug, PartialEq)]
struct MenuNode {
    id: i32,
    /// The label without mnemonics.
    label: String,
    separator: bool,
    enabled: bool,
    visible: bool,
    toggle: Toggle,
    children: Vec<MenuNode>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Toggle {
    None,
    Check(bool),
    Radio(bool),
}

impl MenuNode {
    /// Reads a `(ia{sv}av)` layout.
    fn parse(layout: &Variant) -> Option<Self> {
        if layout.type_().as_str() != "(ia{sv}av)" {
            return None;
        }
        let id = layout.child_value(0).get::<i32>()?;
        let properties = VariantDict::new(Some(&layout.child_value(1)));
        let string = |key| dbus::lookup::<String>(&properties, key).unwrap_or_default();
        let flag = |key| dbus::lookup::<bool>(&properties, key).unwrap_or(true);
        let on = dbus::lookup::<i32>(&properties, "toggle-state") == Some(1);
        let children = layout
            .child_value(2)
            .iter()
            .filter_map(|child| child.as_variant())
            .filter_map(|child| Self::parse(&child))
            .collect();
        Some(Self {
            id,
            label: strip_mnemonics(&string("label")),
            separator: string("type") == "separator",
            enabled: flag("enabled"),
            visible: flag("visible"),
            toggle: match string("toggle-type").as_str() {
                "checkmark" => Toggle::Check(on),
                "radio" => Toggle::Radio(on),
                _ => Toggle::None,
            },
            children,
        })
    }
}

/// Removes dbusmenu's mnemonics from a label: `_` marks the next letter, and `__` is an
/// underscore.
fn strip_mnemonics(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '_' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Builds the menu under `root`, with an action for each item that tells the app it was clicked.
fn build_menu(
    root: &MenuNode,
    bus: &DBusConnection,
    bus_name: &str,
    path: &str,
) -> (gio::MenuModel, gio::SimpleActionGroup) {
    let group = gio::SimpleActionGroup::new();
    let target = Rc::new((bus.clone(), bus_name.to_owned(), path.to_owned()));
    let menu = build_children(&root.children, &group, &target);
    (menu.upcast(), group)
}

/// Builds a menu of `nodes`, with each run between separators as a section.
fn build_children(
    nodes: &[MenuNode],
    group: &gio::SimpleActionGroup,
    target: &Rc<(DBusConnection, String, String)>,
) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    for node in nodes.iter().filter(|node| node.visible) {
        if node.separator {
            if section.n_items() > 0 {
                menu.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }

        // GTK reads mnemonics from menu labels too, so underscores that are left are doubled.
        let label = node.label.replace('_', "__");
        if !node.children.is_empty() {
            let submenu = build_children(&node.children, group, target);
            section.append_item(&gio::MenuItem::new_submenu(Some(&label), &submenu));
            continue;
        }

        let name = format!("item-{}", node.id);
        let action = match node.toggle {
            Toggle::None => gio::SimpleAction::new(&name, None),
            Toggle::Check(on) => gio::SimpleAction::new_stateful(&name, None, &on.to_variant()),
            // A radio item is a string action whose state is its own target when it's on.
            Toggle::Radio(on) => gio::SimpleAction::new_stateful(
                &name,
                Some(glib::VariantTy::STRING),
                &(if on { "on" } else { "" }).to_variant(),
            ),
        };
        action.set_enabled(node.enabled);
        let id = node.id;
        let target = target.clone();
        action.connect_activate(move |_, _| clicked(&target, id));
        group.add_action(&action);

        let item = gio::MenuItem::new(Some(&label), None);
        let detailed = format!("{ACTION_PREFIX}.{name}");
        match node.toggle {
            Toggle::Radio(_) => {
                item.set_action_and_target_value(Some(&detailed), Some(&"on".to_variant()))
            }
            _ => item.set_detailed_action(&detailed),
        }
        section.append_item(&item);
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}

/// Tells the app that a menu item was clicked.
fn clicked(target: &(DBusConnection, String, String), id: i32) {
    let (bus, bus_name, path) = target.clone();
    let parameters = (id, "clicked", 0i32.to_variant(), 0u32).to_variant();
    glib::spawn_future_local(async move {
        let reply = bus
            .call_future(
                Some(&bus_name),
                &path,
                MENU,
                "Event",
                Some(&parameters),
                None,
                DBusCallFlags::NONE,
                -1,
            )
            .await;
        if let Err(err) = reply {
            eprintln!("Couldn't tell {bus_name} its menu was clicked: {err}");
        }
    });
}

/// The StatusNotifierWatcher, which apps register their items with. The shell serves it while
/// no other program does, and takes over when that program leaves.
struct Watcher {
    owner: Cell<Option<OwnerId>>,
    connection: RefCell<Option<DBusConnection>>,
    registration: RefCell<Option<RegistrationId>>,
    /// Whether the shell owns the watcher's name.
    serving: Cell<bool>,
    items: RefCell<Vec<Registered>>,
    hosts: RefCell<Vec<Registered>>,
}

/// An item or host registered with the watcher, which leaves when its bus name does.
struct Registered {
    address: String,
    _watch: dbus::NameWatch,
}

impl Watcher {
    fn start() -> Rc<Self> {
        let this = Rc::new(Self {
            owner: Cell::default(),
            connection: RefCell::default(),
            registration: RefCell::default(),
            serving: Cell::new(false),
            items: RefCell::default(),
            hosts: RefCell::default(),
        });

        let bus_acquired = Rc::downgrade(&this);
        let acquired = Rc::downgrade(&this);
        let lost = Rc::downgrade(&this);
        let owner = gio::bus_own_name(
            BusType::Session,
            WATCHER_NAME,
            BusNameOwnerFlags::NONE,
            move |connection, _| {
                if let Some(this) = bus_acquired.upgrade() {
                    this.register(connection);
                }
            },
            move |_, _| {
                if let Some(this) = acquired.upgrade() {
                    this.serving.set(true);
                }
            },
            move |_, _| {
                if let Some(this) = lost.upgrade() {
                    // Another program is the watcher, or will be once it leaves.
                    this.serving.set(false);
                    this.items.borrow_mut().clear();
                    this.hosts.borrow_mut().clear();
                }
            },
        );
        this.owner.set(Some(owner));
        this
    }

    fn register(self: &Rc<Self>, connection: DBusConnection) {
        let info = DBusNodeInfo::for_xml(WATCHER_XML)
            .ok()
            .and_then(|node| node.lookup_interface(WATCHER));
        let Some(info) = info else {
            eprintln!("The status notifier watcher interface doesn't parse");
            return;
        };

        let call = Rc::downgrade(self);
        let get = Rc::downgrade(self);
        let registration = connection
            .register_object(WATCHER_PATH, &info)
            .method_call(move |_, sender, _, _, method, parameters, invocation| {
                match call.upgrade() {
                    Some(this) => {
                        this.call(sender.unwrap_or_default(), method, &parameters, invocation)
                    }
                    None => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownObject",
                        "The status notifier watcher stopped",
                    ),
                }
            })
            .property(move |_, _, _, _, property| match get.upgrade() {
                Some(this) => this.property(property),
                None => false.to_variant(),
            })
            .build();
        match registration {
            Ok(registration) => {
                *self.registration.borrow_mut() = Some(registration);
                *self.connection.borrow_mut() = Some(connection);
            }
            Err(err) => eprintln!("Couldn't serve the status notifier watcher: {err}"),
        }
    }

    fn call(
        self: &Rc<Self>,
        sender: &str,
        method: &str,
        parameters: &Variant,
        invocation: DBusMethodInvocation,
    ) {
        let Some((service,)) = parameters.get::<(String,)>() else {
            invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", "Expected (s)");
            return;
        };
        match method {
            "RegisterStatusNotifierItem" => {
                self.register_item(item_address(&service, sender));
                invocation.return_value(None);
            }
            "RegisterStatusNotifierHost" => {
                self.register_host(service);
                invocation.return_value(None);
            }
            _ => invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                &format!("No method {method}"),
            ),
        }
    }

    fn property(&self, property: &str) -> Variant {
        match property {
            "RegisteredStatusNotifierItems" => self
                .items
                .borrow()
                .iter()
                .map(|item| item.address.clone())
                .collect::<Vec<_>>()
                .to_variant(),
            "IsStatusNotifierHostRegistered" => (!self.hosts.borrow().is_empty()).to_variant(),
            "ProtocolVersion" => 0i32.to_variant(),
            _ => false.to_variant(),
        }
    }

    fn register_item(self: &Rc<Self>, address: String) {
        let Some(connection) = self.connection.borrow().clone() else {
            return;
        };
        if !self.serving.get() || self.items.borrow().iter().any(|i| i.address == address) {
            return;
        }
        let (bus_name, _) = split_address(&address);
        let weak = Rc::downgrade(self);
        let gone = address.clone();
        let watch = dbus::watch_name_on(
            &connection,
            &bus_name,
            |_| (),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.unregister_item(&gone);
                }
            },
        );
        self.items.borrow_mut().push(Registered {
            address: address.clone(),
            _watch: watch,
        });
        self.emit(
            "StatusNotifierItemRegistered",
            Some(&(address,).to_variant()),
        );
    }

    fn unregister_item(&self, address: &str) {
        let mut items = self.items.borrow_mut();
        let len = items.len();
        items.retain(|item| item.address != address);
        let removed = items.len() != len;
        drop(items);
        if removed {
            self.emit(
                "StatusNotifierItemUnregistered",
                Some(&(address,).to_variant()),
            );
        }
    }

    fn register_host(self: &Rc<Self>, name: String) {
        let Some(connection) = self.connection.borrow().clone() else {
            return;
        };
        if !self.serving.get() || self.hosts.borrow().iter().any(|h| h.address == name) {
            return;
        }
        let weak = Rc::downgrade(self);
        let gone = name.clone();
        let watch = dbus::watch_name_on(
            &connection,
            &name,
            |_| (),
            move || {
                let Some(this) = weak.upgrade() else { return };
                let mut hosts = this.hosts.borrow_mut();
                let len = hosts.len();
                hosts.retain(|host| host.address != gone);
                let removed = hosts.len() != len;
                drop(hosts);
                if removed {
                    this.emit("StatusNotifierHostUnregistered", None);
                }
            },
        );
        self.hosts.borrow_mut().push(Registered {
            address: name,
            _watch: watch,
        });
        self.emit("StatusNotifierHostRegistered", None);
    }

    fn emit(&self, signal: &str, parameters: Option<&Variant>) {
        let Some(connection) = self.connection.borrow().clone() else {
            return;
        };
        if let Err(err) = connection.emit_signal(None, WATCHER_PATH, WATCHER, signal, parameters) {
            eprintln!("Couldn't send {signal}: {err}");
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let connection = self.connection.take();
        if let (Some(connection), Some(registration)) = (connection, self.registration.take()) {
            let _ = connection.unregister_object(registration);
        }
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn node(id: i32, properties: &[(&str, Variant)], children: Vec<Variant>) -> Variant {
        let properties: HashMap<String, Variant> = properties
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect();
        // A `Vec<Variant>` is an `av`, so each child goes in as a `v`.
        (id, properties, children).to_variant()
    }

    #[test]
    fn layouts_parse_into_nodes() {
        let layout = node(
            0,
            &[("children-display", "submenu".to_variant())],
            vec![
                node(1, &[("label", "_Open".to_variant())], vec![]),
                node(2, &[("type", "separator".to_variant())], vec![]),
                node(
                    3,
                    &[
                        ("label", "Mute".to_variant()),
                        ("toggle-type", "checkmark".to_variant()),
                        ("toggle-state", 1i32.to_variant()),
                        ("enabled", false.to_variant()),
                    ],
                    vec![],
                ),
                node(
                    4,
                    &[("label", "Mode".to_variant())],
                    vec![node(
                        5,
                        &[
                            ("label", "Fast".to_variant()),
                            ("toggle-type", "radio".to_variant()),
                            ("toggle-state", 0i32.to_variant()),
                        ],
                        vec![],
                    )],
                ),
                node(6, &[("visible", false.to_variant())], vec![]),
            ],
        );

        let root = MenuNode::parse(&layout).unwrap();
        assert_eq!(root.id, 0);
        let children = &root.children;
        assert_eq!(children.len(), 5);

        assert_eq!(children[0].label, "Open");
        assert!(children[0].enabled && children[0].visible && !children[0].separator);
        assert!(children[1].separator);
        assert_eq!(children[2].toggle, Toggle::Check(true));
        assert!(!children[2].enabled);
        assert_eq!(children[3].children.len(), 1);
        assert_eq!(children[3].children[0].label, "Fast");
        assert_eq!(children[3].children[0].toggle, Toggle::Radio(false));
        assert!(!children[4].visible);
    }

    #[test]
    fn a_layout_of_the_wrong_type_is_none() {
        assert_eq!(MenuNode::parse(&"menu".to_variant()), None);
    }

    #[test]
    fn mnemonics_are_stripped() {
        assert_eq!(strip_mnemonics("_File"), "File");
        assert_eq!(strip_mnemonics("Save __as"), "Save _as");
        assert_eq!(strip_mnemonics("trailing_"), "trailing");
    }

    #[test]
    fn addresses_split_into_name_and_path() {
        assert_eq!(
            split_address(":1.45/org/ayatana/NotificationItem/x"),
            (
                String::from(":1.45"),
                String::from("/org/ayatana/NotificationItem/x")
            )
        );
        assert_eq!(
            split_address("org.kde.StatusNotifierItem-3-1"),
            (
                String::from("org.kde.StatusNotifierItem-3-1"),
                String::from("/StatusNotifierItem")
            )
        );
    }

    #[test]
    fn items_register_by_name_or_path() {
        assert_eq!(
            item_address("org.kde.StatusNotifierItem-3-1", ":1.9"),
            "org.kde.StatusNotifierItem-3-1/StatusNotifierItem"
        );
        assert_eq!(
            item_address("/org/ayatana/NotificationItem/x", ":1.9"),
            ":1.9/org/ayatana/NotificationItem/x"
        );
    }

    #[test]
    fn the_largest_pixmap_wins() {
        let pixmaps = vec![
            (16i32, 16i32, vec![0u8; 16 * 16 * 4]),
            (32, 32, vec![0u8; 32 * 32 * 4]),
            // Too little data for its size.
            (64, 64, vec![0u8; 4]),
        ]
        .to_variant();
        let pixmap = Pixmap::largest(&pixmaps).unwrap();
        assert_eq!((pixmap.width, pixmap.height), (32, 32));
        assert_eq!(
            Pixmap::largest(&Vec::<(i32, i32, Vec<u8>)>::new().to_variant()),
            None
        );
    }

    #[test]
    fn properties_pick_the_attention_icon() {
        let dict = VariantDict::new(None);
        dict.insert_value("Id", &"app".to_variant());
        dict.insert_value("Status", &"NeedsAttention".to_variant());
        dict.insert_value("IconName", &"app-symbolic".to_variant());
        dict.insert_value("AttentionIconName", &"app-alert".to_variant());
        dict.insert_value(
            "Menu",
            &glib::variant::ObjectPath::try_from("/Menu")
                .unwrap()
                .to_variant(),
        );
        let properties = Properties::parse(&dict);
        assert_eq!(properties.menu.as_deref(), Some("/Menu"));

        let item = properties.into_item(String::from(":1.2/StatusNotifierItem"));
        assert_eq!(item.title, "app");
        assert_eq!(item.status, Status::NeedsAttention);
        let Some(TrayIcon::Name { name, theme_path }) = item.icon else {
            panic!("no icon name");
        };
        assert_eq!((name.as_str(), theme_path), ("app-alert", None));
    }
}
