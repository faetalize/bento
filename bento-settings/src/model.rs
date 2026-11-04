//! What the app shows: sections of categories in the sidebar, each a page of setting groups.
//!
//! A setting names its place in the config as a dotted path (`layout.focus-ring.width`), reads its
//! current value from the parsed bento config, and knows how to write itself into config.kdl.
//! Adding a setting to a page is one builder chain; see `pages/appearance.rs`.

use niri_config::{Color, Config};

use crate::kdl::{self, Document, Value};

pub type Getter<T> = fn(&Config) -> T;

/// A titled group of categories in the sidebar.
pub struct Section {
    pub title: &'static str,
    pub categories: Vec<Category>,
}

impl Section {
    pub fn new(title: &'static str, categories: Vec<Category>) -> Self {
        Self { title, categories }
    }
}

pub struct Category {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    /// Words search should match besides the title.
    pub keywords: &'static [&'static str],
    pub content: Content,
}

pub enum Content {
    /// Groups of settings, built into rows automatically.
    Settings(Vec<Group>),
    /// A page that builds its own widgets, listing what search should find on it.
    Custom {
        build: fn(&crate::page::PageContext) -> gtk::Widget,
        search: Vec<SearchItem>,
    },
    /// A page that isn't in the app yet.
    Planned,
}

/// Something on a custom page that search can jump to.
pub struct SearchItem {
    pub anchor: &'static str,
    pub title: &'static str,
    pub keywords: &'static [&'static str],
}

impl Category {
    fn new(id: &'static str, title: &'static str, content: Content) -> Self {
        Self {
            id,
            title,
            icon: "ph-gear-six-symbolic",
            keywords: &[],
            content,
        }
    }

    pub fn settings(id: &'static str, title: &'static str) -> Self {
        Self::new(id, title, Content::Settings(Vec::new()))
    }

    pub fn custom(
        id: &'static str,
        title: &'static str,
        build: fn(&crate::page::PageContext) -> gtk::Widget,
    ) -> Self {
        Self::new(
            id,
            title,
            Content::Custom {
                build,
                search: Vec::new(),
            },
        )
    }

    pub fn planned(id: &'static str, title: &'static str) -> Self {
        Self::new(id, title, Content::Planned)
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = icon;
        self
    }

    pub fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }

    pub fn group(mut self, group: Group) -> Self {
        match &mut self.content {
            Content::Settings(groups) => groups.push(group),
            _ => panic!("only settings pages have groups"),
        }
        self
    }

    pub fn search_item(
        mut self,
        anchor: &'static str,
        title: &'static str,
        keywords: &'static [&'static str],
    ) -> Self {
        match &mut self.content {
            Content::Custom { search, .. } => search.push(SearchItem {
                anchor,
                title,
                keywords,
            }),
            _ => panic!("only custom pages have search items"),
        }
        self
    }
}

pub struct Group {
    pub title: Option<&'static str>,
    pub settings: Vec<Setting>,
}

impl Group {
    pub fn new(title: &'static str) -> Self {
        Self {
            title: Some(title),
            settings: Vec::new(),
        }
    }

    pub fn untitled() -> Self {
        Self {
            title: None,
            settings: Vec::new(),
        }
    }

    pub fn setting(mut self, setting: Setting) -> Self {
        self.settings.push(setting);
        self
    }
}

/// How a switch is spelled in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolStyle {
    /// `numlock` for true, `numlock false` for false.
    Flag,
    /// `draw-behind-window true` or `draw-behind-window false`.
    Arg,
    /// `tap` for true, nothing for false.
    Presence,
    /// The path is a section, which gets an `on` or `off` child.
    OnOff,
}

pub enum Control {
    Switch {
        get: Getter<bool>,
        style: BoolStyle,
        /// The switch shows the opposite of what the config says, like "Middle-click paste" for
        /// `disable-primary`.
        inverted: bool,
    },
    Number {
        get: Getter<f64>,
        min: f64,
        max: f64,
        step: f64,
        digits: u32,
    },
    /// One of a fixed set of words. `None` is "leave it unset".
    Choice {
        get: Getter<Option<&'static str>>,
        choices: Vec<(Option<&'static str>, &'static str)>,
    },
    Text {
        get: Getter<String>,
    },
    Color {
        get: Getter<Color>,
    },
}

/// The value a row hands back to its setting.
#[derive(Debug, Clone)]
pub enum Input {
    Bool(bool),
    Number(f64),
    Choice(Option<&'static str>),
    Text(String),
    Color(Color),
}

pub struct Setting {
    pub path: &'static str,
    pub title: &'static str,
    pub keywords: &'static [&'static str],
    pub control: Control,
    /// Settings that only matter while this switch is on. The switch becomes an expander row
    /// holding them.
    pub children: Vec<Setting>,
}

impl Setting {
    fn new(title: &'static str, path: &'static str, control: Control) -> Self {
        Self {
            path,
            title,
            keywords: &[],
            control,
            children: Vec::new(),
        }
    }

    pub fn switch(title: &'static str, path: &'static str, get: Getter<bool>) -> Self {
        Self::new(
            title,
            path,
            Control::Switch {
                get,
                style: BoolStyle::Flag,
                inverted: false,
            },
        )
    }

    pub fn number(title: &'static str, path: &'static str, get: Getter<f64>) -> Self {
        Self::new(
            title,
            path,
            Control::Number {
                get,
                min: 0.,
                max: 100.,
                step: 1.,
                digits: 0,
            },
        )
    }

    pub fn choice(
        title: &'static str,
        path: &'static str,
        get: Getter<Option<&'static str>>,
    ) -> Self {
        Self::new(
            title,
            path,
            Control::Choice {
                get,
                choices: Vec::new(),
            },
        )
    }

    pub fn text(title: &'static str, path: &'static str, get: Getter<String>) -> Self {
        Self::new(title, path, Control::Text { get })
    }

    pub fn color(title: &'static str, path: &'static str, get: Getter<Color>) -> Self {
        Self::new(title, path, Control::Color { get })
    }

    pub fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }

    /// Puts settings under this switch, shown when it's on.
    pub fn expands(mut self, children: Vec<Setting>) -> Self {
        assert!(
            matches!(self.control, Control::Switch { .. }),
            "only switches expand"
        );
        self.children = children;
        self
    }

    pub fn style(mut self, new_style: BoolStyle) -> Self {
        match &mut self.control {
            Control::Switch { style, .. } => *style = new_style,
            _ => panic!("only switches have a style"),
        }
        self
    }

    pub fn inverted(mut self) -> Self {
        match &mut self.control {
            Control::Switch { inverted, .. } => *inverted = true,
            _ => panic!("only switches can be inverted"),
        }
        self
    }

    pub fn range(mut self, new_min: f64, new_max: f64, new_step: f64) -> Self {
        match &mut self.control {
            Control::Number { min, max, step, .. } => {
                *min = new_min;
                *max = new_max;
                *step = new_step;
            }
            _ => panic!("only numbers have a range"),
        }
        self
    }

    /// Decimal places. Settings with none are written as integers, which some of bento's
    /// settings require.
    pub fn digits(mut self, new_digits: u32) -> Self {
        match &mut self.control {
            Control::Number { digits, .. } => *digits = new_digits,
            _ => panic!("only numbers have digits"),
        }
        self
    }

    pub fn option(mut self, value: Option<&'static str>, label: &'static str) -> Self {
        match &mut self.control {
            Control::Choice { choices, .. } => choices.push((value, label)),
            _ => panic!("only choices have options"),
        }
        self
    }

    pub fn path_parts(&self) -> Vec<&'static str> {
        kdl::split(self.path)
    }

    /// Writes a value into the config.
    pub fn write(&self, doc: &mut Document, input: Input) {
        let path = self.path_parts();
        match (&self.control, input) {
            (
                Control::Switch {
                    style, inverted, ..
                },
                Input::Bool(on),
            ) => {
                let value = on != *inverted;
                match style {
                    BoolStyle::Flag if value => doc.set(&path, Vec::new()),
                    BoolStyle::Flag | BoolStyle::Arg => doc.set(&path, vec![Value::Bool(value)]),
                    BoolStyle::Presence if value => {
                        if doc.get(&path).is_none() {
                            doc.set(&path, Vec::new());
                        }
                    }
                    BoolStyle::Presence => {
                        doc.remove(&path);
                    }
                    BoolStyle::OnOff => {
                        let [mut on, mut off] = [path.clone(), path];
                        on.push("on");
                        off.push("off");
                        doc.remove(&on);
                        doc.remove(&off);
                        doc.set(if value { &on } else { &off }, Vec::new());
                    }
                }
            }
            (Control::Number { digits, .. }, Input::Number(x)) => {
                let value = if *digits == 0 {
                    Value::Int(x.round() as i64)
                } else {
                    Value::Float(round_to(x, *digits))
                };
                doc.set(&path, vec![value]);
            }
            (Control::Choice { .. }, Input::Choice(Some(word))) => {
                doc.set(&path, vec![Value::Str(word.to_owned())]);
            }
            // Leaving a choice or text unset gives the default.
            (Control::Choice { .. }, Input::Choice(None)) => {
                doc.remove(&path);
            }
            (Control::Text { .. }, Input::Text(text)) if text.is_empty() => {
                doc.remove(&path);
            }
            (Control::Text { .. }, Input::Text(text)) => {
                doc.set(&path, vec![Value::Str(text)]);
            }
            (Control::Color { .. }, Input::Color(color)) => {
                doc.set(&path, vec![Value::Str(color_to_hex(color))]);
            }
            (_, input) => panic!("{input:?} doesn't fit the setting at {}", self.path),
        }
    }
}

fn round_to(x: f64, digits: u32) -> f64 {
    let factor = 10f64.powi(digits as i32);
    (x * factor).round() / factor
}

pub fn color_to_hex(color: Color) -> String {
    let [r, g, b, a] = color.to_array_unpremul().map(|x| (x * 255.).round() as u8);
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}
