//! The notification server: apps' notifications through `org.freedesktop.Notifications`, version
//! 1.2 of the spec.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gio::prelude::*;
use gio::{
    BusNameOwnerFlags, BusType, DBusConnection, DBusMethodInvocation, DBusNodeInfo, OwnerId,
    RegistrationId,
};
use glib::{Variant, VariantDict};
use gtk::gdk;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

const XML: &str = r#"
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="Notify">
      <arg type="s" name="app_name" direction="in"/>
      <arg type="u" name="replaces_id" direction="in"/>
      <arg type="s" name="app_icon" direction="in"/>
      <arg type="s" name="summary" direction="in"/>
      <arg type="s" name="body" direction="in"/>
      <arg type="as" name="actions" direction="in"/>
      <arg type="a{sv}" name="hints" direction="in"/>
      <arg type="i" name="expire_timeout" direction="in"/>
      <arg type="u" name="id" direction="out"/>
    </method>
    <method name="CloseNotification">
      <arg type="u" name="id" direction="in"/>
    </method>
    <method name="GetCapabilities">
      <arg type="as" name="capabilities" direction="out"/>
    </method>
    <method name="GetServerInformation">
      <arg type="s" name="name" direction="out"/>
      <arg type="s" name="vendor" direction="out"/>
      <arg type="s" name="version" direction="out"/>
      <arg type="s" name="spec_version" direction="out"/>
    </method>
    <signal name="NotificationClosed">
      <arg type="u" name="id"/>
      <arg type="u" name="reason"/>
    </signal>
    <signal name="ActionInvoked">
      <arg type="u" name="id"/>
      <arg type="s" name="action_key"/>
    </signal>
    <signal name="ActivationToken">
      <arg type="u" name="id"/>
      <arg type="s" name="activation_token"/>
    </signal>
  </interface>
</node>
"#;

const CAPABILITIES: &[&str] = &[
    "body",
    "body-markup",
    "actions",
    "icon-static",
    "persistence",
];

/// Why a notification closed, as the spec numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

/// A notification an app sent.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    /// An icon name or a `file://` URI, or empty.
    pub app_icon: String,
    pub summary: String,
    /// The text, which can hold the markup the spec allows; [`Notification::body_markup`] makes
    /// it Pango markup.
    pub body: String,
    /// The buttons, as action keys and labels. The key `default` is clicking the notification.
    pub actions: Vec<(String, String)>,
    /// 0 is low, 1 normal and 2 critical.
    pub urgency: u8,
    /// How long to show it, in milliseconds: -1 for as long as the shell likes, 0 for until it's
    /// dismissed.
    pub expire_timeout: i32,
    pub image: Option<gdk::Texture>,
}

impl Notification {
    /// The body as Pango markup: bold, italic, underline and links are kept, other tags dropped,
    /// and stray `&` and `<` escaped.
    pub fn body_markup(&self) -> String {
        pango_markup(&self.body)
    }
}

/// What happens to notifications.
#[derive(Debug, Clone)]
pub enum Event {
    Shown(Notification),
    /// A notification that's shown changed; it has the same id.
    Replaced(Notification),
    /// A notification went away, whether it expired, was dismissed, invoked or closed by its app.
    Closed(u32),
}

/// What the server keeps about a notification that's shown.
struct Shown {
    /// Whether it stays after one of its actions is invoked.
    resident: bool,
}

/// The notification server. It owns `org.freedesktop.Notifications` unless another program
/// already does, in which case it stays inactive.
pub struct Notifications {
    owner: Cell<Option<OwnerId>>,
    connection: RefCell<Option<DBusConnection>>,
    registration: RefCell<Option<RegistrationId>>,
    shown: RefCell<HashMap<u32, Shown>>,
    next_id: Cell<u32>,
    on_event: Box<dyn Fn(&Event)>,
}

impl Notifications {
    pub fn start(on_event: impl Fn(&Event) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            owner: Cell::default(),
            connection: RefCell::default(),
            registration: RefCell::default(),
            shown: RefCell::default(),
            next_id: Cell::new(1),
            on_event: Box::new(on_event),
        });

        let acquired = Rc::downgrade(&this);
        let lost = Rc::downgrade(&this);
        let owner = gio::bus_own_name(
            BusType::Session,
            NAME,
            BusNameOwnerFlags::DO_NOT_QUEUE,
            move |connection, _| {
                if let Some(this) = acquired.upgrade() {
                    this.register(connection);
                }
            },
            |_, _| (),
            move |_, _| {
                eprintln!("Another program shows notifications, so the shell won't");
                if let Some(this) = lost.upgrade() {
                    this.stop();
                }
            },
        );
        this.owner.set(Some(owner));
        this
    }

    /// Closes a notification as the user dismissing it.
    pub fn dismiss(&self, id: u32) {
        self.close(id, Reason::Dismissed);
    }

    /// Closes a notification whose time ran out.
    pub fn expire(&self, id: u32) {
        self.close(id, Reason::Expired);
    }

    /// Tells the app that the user picked one of a notification's actions, and closes it unless
    /// it's resident. With an activation `token`, the app may bring its window forward, which
    /// the spec has it hear just before the action.
    pub fn invoke(&self, id: u32, action_key: &str, token: Option<&str>) {
        let Some(resident) = self.shown.borrow().get(&id).map(|shown| shown.resident) else {
            return;
        };
        if let Some(token) = token.filter(|token| !token.is_empty()) {
            self.emit("ActivationToken", &(id, token).to_variant());
        }
        self.emit("ActionInvoked", &(id, action_key).to_variant());
        if !resident {
            self.close(id, Reason::Dismissed);
        }
    }

    fn register(self: &Rc<Self>, connection: DBusConnection) {
        let info = DBusNodeInfo::for_xml(XML)
            .ok()
            .and_then(|node| node.lookup_interface(INTERFACE));
        let Some(info) = info else {
            eprintln!("The notification interface doesn't parse");
            return;
        };

        let weak = Rc::downgrade(self);
        let registration = connection
            .register_object(PATH, &info)
            .method_call(
                move |_, _, _, _, method, parameters, invocation| match weak.upgrade() {
                    Some(this) => this.call(method, &parameters, invocation),
                    None => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownObject",
                        "The notification server stopped",
                    ),
                },
            )
            .build();
        match registration {
            Ok(registration) => {
                *self.registration.borrow_mut() = Some(registration);
                *self.connection.borrow_mut() = Some(connection);
            }
            Err(err) => eprintln!("Couldn't serve notifications: {err}"),
        }
    }

    /// Stops serving, after losing the name.
    fn stop(&self) {
        let connection = self.connection.take();
        if let (Some(connection), Some(registration)) = (connection, self.registration.take()) {
            let _ = connection.unregister_object(registration);
        }
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
    }

    fn call(&self, method: &str, parameters: &Variant, invocation: DBusMethodInvocation) {
        match method {
            "Notify" => match self.notify(parameters) {
                Some(id) => invocation.return_value(Some(&(id,).to_variant())),
                None => invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.InvalidArgs",
                    "Notify takes (susssasa{sv}i)",
                ),
            },
            "CloseNotification" => {
                if let Some((id,)) = parameters.get::<(u32,)>() {
                    self.close(id, Reason::Closed);
                }
                invocation.return_value(None);
            }
            "GetCapabilities" => {
                invocation.return_value(Some(&(CAPABILITIES,).to_variant()));
            }
            "GetServerInformation" => {
                let info = ("bento-shell", "bento", env!("CARGO_PKG_VERSION"), "1.2");
                invocation.return_value(Some(&info.to_variant()));
            }
            _ => invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                &format!("No method {method}"),
            ),
        }
    }

    fn notify(&self, parameters: &Variant) -> Option<u32> {
        if parameters.type_().as_str() != "(susssasa{sv}i)" {
            return None;
        }
        let app_name: String = parameters.child_value(0).get()?;
        let replaces_id: u32 = parameters.child_value(1).get()?;
        let mut app_icon: String = parameters.child_value(2).get()?;
        let summary: String = parameters.child_value(3).get()?;
        let body: String = parameters.child_value(4).get()?;
        let actions: Vec<String> = parameters.child_value(5).get()?;
        let hints = Hints::parse(&VariantDict::new(Some(&parameters.child_value(6))));
        let expire_timeout: i32 = parameters.child_value(7).get()?;

        let id = if replaces_id != 0 {
            self.next_id
                .set(self.next_id.get().max(replaces_id.wrapping_add(1)).max(1));
            replaces_id
        } else {
            self.new_id()
        };

        let image = match hints.image {
            Some(Image::Pixels(pixels)) => Some(pixels.texture()),
            Some(Image::Path(path)) => load_image(&path).or_else(|| {
                // `image-path` can name an icon too.
                if app_icon.is_empty() {
                    app_icon = path;
                }
                None
            }),
            None => None,
        };

        let notification = Notification {
            id,
            app_name,
            app_icon,
            summary,
            body,
            actions: pairs(actions),
            urgency: hints.urgency,
            expire_timeout,
            image,
        };
        let replaced = self
            .shown
            .borrow_mut()
            .insert(
                id,
                Shown {
                    resident: hints.resident,
                },
            )
            .is_some();
        (self.on_event)(&if replaced {
            Event::Replaced(notification)
        } else {
            Event::Shown(notification)
        });
        Some(id)
    }

    fn new_id(&self) -> u32 {
        loop {
            let id = self.next_id.get();
            self.next_id.set(id.wrapping_add(1).max(1));
            if !self.shown.borrow().contains_key(&id) {
                return id;
            }
        }
    }

    fn close(&self, id: u32, reason: Reason) {
        if self.shown.borrow_mut().remove(&id).is_none() {
            return;
        }
        self.emit("NotificationClosed", &(id, reason as u32).to_variant());
        (self.on_event)(&Event::Closed(id));
    }

    fn emit(&self, signal: &str, parameters: &Variant) {
        let Some(connection) = self.connection.borrow().clone() else {
            return;
        };
        if let Err(err) = connection.emit_signal(None, PATH, INTERFACE, signal, Some(parameters)) {
            eprintln!("Couldn't send {signal}: {err}");
        }
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The hints the server uses.
#[derive(Debug, PartialEq)]
struct Hints {
    urgency: u8,
    resident: bool,
    image: Option<Image>,
}

#[derive(Debug, PartialEq)]
enum Image {
    Pixels(Pixels),
    Path(String),
}

impl Hints {
    fn parse(hints: &VariantDict) -> Self {
        let lookup = |key: &str| hints.lookup_value(key, None);
        // The spec ranks the image hints, and names some differently in older versions.
        let pixels = ["image-data", "image_data"]
            .into_iter()
            .find_map(|key| lookup(key).and_then(|value| Pixels::parse(&value)))
            .map(Image::Pixels);
        let path = ["image-path", "image_path"]
            .into_iter()
            .find_map(|key| lookup(key).and_then(|value| value.get::<String>()))
            .filter(|path| !path.is_empty())
            .map(Image::Path);
        let icon = lookup("icon_data")
            .and_then(|value| Pixels::parse(&value))
            .map(Image::Pixels);

        Self {
            urgency: lookup("urgency")
                .and_then(|value| value.get::<u8>())
                .unwrap_or(1)
                .min(2),
            resident: lookup("resident")
                .and_then(|value| value.get::<bool>())
                .unwrap_or(false),
            image: pixels.or(path).or(icon),
        }
    }
}

/// An image sent as pixels, in the spec's `(iiibiiay)`.
#[derive(Debug, PartialEq)]
struct Pixels {
    width: i32,
    height: i32,
    stride: usize,
    has_alpha: bool,
    data: Vec<u8>,
}

impl Pixels {
    fn parse(value: &Variant) -> Option<Self> {
        let (width, height, stride, has_alpha, bits, channels, data) =
            value.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()?;
        let expected_channels = if has_alpha { 4 } else { 3 };
        if width <= 0 || height <= 0 || bits != 8 || channels != expected_channels {
            return None;
        }
        let stride = usize::try_from(stride).ok()?;
        let row = usize::try_from(width).ok()? * usize::try_from(channels).ok()?;
        let rows = usize::try_from(height).ok()?;
        if stride < row || data.len() < stride * (rows - 1) + row {
            return None;
        }
        Some(Self {
            width,
            height,
            stride,
            has_alpha,
            data,
        })
    }

    fn texture(self) -> gdk::Texture {
        let format = if self.has_alpha {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        let bytes = glib::Bytes::from_owned(self.data);
        let texture = gdk::MemoryTexture::new(self.width, self.height, format, &bytes, self.stride);
        texture.upcast()
    }
}

/// Loads an image from a path or a `file://` URI.
fn load_image(path: &str) -> Option<gdk::Texture> {
    let file = if path.starts_with("file://") {
        gio::File::for_uri(path)
    } else if path.starts_with('/') {
        gio::File::for_path(path)
    } else {
        return None;
    };
    match gdk::Texture::from_file(&file) {
        Ok(texture) => Some(texture),
        Err(err) => {
            eprintln!("Couldn't load a notification's image {path}: {err}");
            None
        }
    }
}

/// Pairs up the spec's flat list of action keys and labels.
fn pairs(actions: Vec<String>) -> Vec<(String, String)> {
    let mut actions = actions.into_iter();
    let mut pairs = Vec::new();
    while let (Some(key), Some(label)) = (actions.next(), actions.next()) {
        pairs.push((key, label));
    }
    pairs
}

/// Turns the spec's body markup into Pango markup: `b`, `i`, `u` and `a` are kept, other tags
/// dropped, and text that would break Pango's parser escaped.
fn pango_markup(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        match c {
            '<' => match rest.find('>').filter(|_| starts_tag(rest)) {
                Some(end) => {
                    let tag = &rest[..=end];
                    if is_kept_tag(tag) {
                        out.push_str(tag);
                    }
                    rest = &rest[end + 1..];
                    continue;
                }
                None => out.push_str("&lt;"),
            },
            '>' => out.push_str("&gt;"),
            '&' if starts_with_entity(rest) => out.push('&'),
            '&' => out.push_str("&amp;"),
            c => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Whether the `<` that `text` starts with opens a tag, rather than being a less-than sign.
fn starts_tag(text: &str) -> bool {
    text[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/')
}

fn is_kept_tag(tag: &str) -> bool {
    let inner = tag[1..tag.len() - 1].trim();
    let inner = inner.strip_prefix('/').unwrap_or(inner);
    let name = inner.split_whitespace().next().unwrap_or("");
    match name {
        "b" | "i" | "u" => inner == name,
        "a" => true,
        _ => false,
    }
}

fn starts_with_entity(text: &str) -> bool {
    let Some(end) = text.find(';') else {
        return false;
    };
    let name = &text[1..end];
    match name.strip_prefix('#') {
        Some(code) => match code.strip_prefix(['x', 'X']) {
            Some(hex) => !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()),
            None => !code.is_empty() && code.chars().all(|c| c.is_ascii_digit()),
        },
        None => matches!(name, "amp" | "lt" | "gt" | "quot" | "apos"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(entries: &[(&str, Variant)]) -> Hints {
        let dict = VariantDict::new(None);
        for (key, value) in entries {
            dict.insert_value(key, value);
        }
        Hints::parse(&dict)
    }

    fn pixels(width: i32, height: i32, stride: i32, alpha: bool, len: usize) -> Variant {
        let channels = if alpha { 4 } else { 3 };
        (width, height, stride, alpha, 8, channels, vec![0u8; len]).to_variant()
    }

    #[test]
    fn hints_default_to_normal_urgency_without_an_image() {
        assert_eq!(
            hints(&[]),
            Hints {
                urgency: 1,
                resident: false,
                image: None,
            }
        );
    }

    #[test]
    fn urgency_and_resident_are_read() {
        let parsed = hints(&[
            ("urgency", 2u8.to_variant()),
            ("resident", true.to_variant()),
        ]);
        assert_eq!((parsed.urgency, parsed.resident), (2, true));
        assert_eq!(hints(&[("urgency", 9u8.to_variant())]).urgency, 2);
    }

    #[test]
    fn image_data_comes_before_a_path_and_icon_data() {
        let parsed = hints(&[
            ("icon_data", pixels(1, 1, 3, false, 3)),
            ("image-path", "/tmp/a.png".to_variant()),
            ("image-data", pixels(2, 2, 8, true, 16)),
        ]);
        let Some(Image::Pixels(image)) = parsed.image else {
            panic!("no pixels");
        };
        assert_eq!((image.width, image.height, image.stride), (2, 2, 8));
        assert!(image.has_alpha);

        let parsed = hints(&[
            ("icon_data", pixels(1, 1, 3, false, 3)),
            ("image_path", "/tmp/a.png".to_variant()),
        ]);
        assert_eq!(parsed.image, Some(Image::Path(String::from("/tmp/a.png"))));

        let parsed = hints(&[("icon_data", pixels(1, 1, 3, false, 3))]);
        assert!(matches!(parsed.image, Some(Image::Pixels(_))));
    }

    #[test]
    fn malformed_pixels_are_ignored() {
        // Too little data for the size.
        assert_eq!(Pixels::parse(&pixels(4, 4, 16, true, 20)), None);
        // A stride shorter than a row.
        assert_eq!(Pixels::parse(&pixels(4, 1, 8, true, 16)), None);
        // The last row doesn't need its padding.
        assert!(Pixels::parse(&pixels(3, 2, 12, false, 21)).is_some());
        assert_eq!(Pixels::parse(&"no".to_variant()), None);
    }

    #[test]
    fn actions_pair_up() {
        let actions = ["default", "Open", "reply", "Reply", "odd"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            pairs(actions),
            [
                (String::from("default"), String::from("Open")),
                (String::from("reply"), String::from("Reply")),
            ]
        );
    }

    #[test]
    fn body_markup_becomes_pango_markup() {
        assert_eq!(
            pango_markup("<b>hi</b> & <i>you</i>"),
            "<b>hi</b> &amp; <i>you</i>"
        );
        assert_eq!(
            pango_markup(r#"<a href="https://x.org">x</a><img src="a.png" alt="a"/>"#),
            r#"<a href="https://x.org">x</a>"#
        );
        assert_eq!(
            pango_markup("a &lt; b &#60; c &#x3c;"),
            "a &lt; b &#60; c &#x3c;"
        );
        assert_eq!(pango_markup("1 < 2 > 0"), "1 &lt; 2 &gt; 0");
        assert_eq!(pango_markup("<br/>line<p>x</p>"), "linex");
    }
}
