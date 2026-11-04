//! What the app shows: sections of categories in the sidebar, each a page of setting groups.
//!
//! A setting names its place in the config as a dotted path (`layout.focus-ring.width`), reads its
//! current value from the parsed bento config, and knows how to write itself into settings.kdl.
//! Adding a setting to a page is one builder chain; see `pages/appearance.rs`.

use niri_config::{Color, Config};

use crate::kdl::{self, Node, Value};

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
    pub description: &'static str,
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
            icon: "preferences-system-symbolic",
            description: "",
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

    pub fn description(mut self, description: &'static str) -> Self {
        self.description = description;
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
    pub title: &'static str,
    pub description: Option<&'static str>,
    pub settings: Vec<Setting>,
}

impl Group {
    pub fn new(title: &'static str) -> Self {
        Self {
            title,
            description: None,
            settings: Vec::new(),
        }
    }

    pub fn description(mut self, description: &'static str) -> Self {
        self.description = Some(description);
        self
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
    /// `tap` for true, nothing for false. Only inside sections that bento replaces as a whole.
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
    pub subtitle: Option<&'static str>,
    pub keywords: &'static [&'static str],
    pub control: Control,
    pub enabled_if: Option<Getter<bool>>,
}

impl Setting {
    fn new(title: &'static str, path: &'static str, control: Control) -> Self {
        Self {
            path,
            title,
            subtitle: None,
            keywords: &[],
            control,
            enabled_if: None,
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

    pub fn subtitle(mut self, subtitle: &'static str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    pub fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }

    /// Greys the row out unless the condition holds, like a width while its border is off.
    pub fn enabled_if(mut self, condition: Getter<bool>) -> Self {
        self.enabled_if = Some(condition);
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

    /// Writes a value into the settings document.
    pub fn write(&self, doc: &mut Vec<Node>, input: Input) {
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
                    BoolStyle::Flag if value => kdl::set_args(doc, &path, Vec::new()),
                    BoolStyle::Flag | BoolStyle::Arg => {
                        kdl::set_args(doc, &path, vec![Value::Bool(value)])
                    }
                    BoolStyle::Presence if value => kdl::set_args(doc, &path, Vec::new()),
                    BoolStyle::Presence => {
                        kdl::remove(doc, &path);
                    }
                    BoolStyle::OnOff => {
                        let section = kdl::ensure(doc, &path);
                        section
                            .children
                            .retain(|c| c.name != "on" && c.name != "off");
                        section
                            .children
                            .insert(0, Node::new(if value { "on" } else { "off" }));
                    }
                }
            }
            (Control::Number { digits, .. }, Input::Number(x)) => {
                let value = if *digits == 0 {
                    Value::Int(x.round() as i64)
                } else {
                    Value::Float(round_to(x, *digits))
                };
                kdl::set_args(doc, &path, vec![value]);
            }
            (Control::Choice { .. }, Input::Choice(Some(word))) => {
                kdl::set_args(doc, &path, vec![Value::Str(word.to_owned())]);
            }
            (Control::Choice { .. }, Input::Choice(None)) => {
                kdl::remove(doc, &path);
            }
            (Control::Text { .. }, Input::Text(text)) if text.is_empty() => {
                kdl::remove(doc, &path);
            }
            (Control::Text { .. }, Input::Text(text)) => {
                kdl::set_args(doc, &path, vec![Value::Str(text)]);
            }
            (Control::Color { .. }, Input::Color(color)) => {
                kdl::set_args(doc, &path, vec![Value::Str(color_to_hex(color))]);
            }
            (_, input) => panic!("{input:?} doesn't fit the setting at {}", self.path),
        }
    }

    /// Whether settings.kdl sets this setting.
    pub fn is_set(&self, doc: &[Node]) -> bool {
        let path = self.path_parts();
        match self.control {
            Control::Switch {
                style: BoolStyle::OnOff,
                ..
            } => kdl::get(doc, &path).is_some_and(|section| {
                section
                    .children
                    .iter()
                    .any(|c| c.name == "on" || c.name == "off")
            }),
            _ => kdl::get(doc, &path).is_some(),
        }
    }

    /// Removes this setting from settings.kdl, so it falls back to config.kdl or the default.
    pub fn reset(&self, doc: &mut Vec<Node>) {
        let path = self.path_parts();
        match self.control {
            Control::Switch {
                style: BoolStyle::OnOff,
                ..
            } => {
                let mut on = path.clone();
                on.push("on");
                let mut off = path;
                off.push("off");
                kdl::remove(doc, &on);
                kdl::remove(doc, &off);
            }
            _ => {
                kdl::remove(doc, &path);
            }
        }
    }

    /// Whether resetting means anything. A switch spelled by presence alone has no "unset".
    pub fn can_reset(&self) -> bool {
        !matches!(
            self.control,
            Control::Switch {
                style: BoolStyle::Presence,
                ..
            }
        )
    }

    /// The value settings.kdl asks for, to tell when something later in the config overrides it.
    pub fn wanted(&self, doc: &[Node]) -> Option<Input> {
        let path = self.path_parts();
        let node = kdl::get(doc, &path);
        Some(match &self.control {
            Control::Switch {
                style, inverted, ..
            } => {
                let value = match style {
                    BoolStyle::Flag | BoolStyle::Arg => match node?.arg() {
                        None => true,
                        Some(Value::Bool(x)) => *x,
                        Some(_) => return None,
                    },
                    BoolStyle::Presence => node.is_some(),
                    BoolStyle::OnOff => {
                        let section = node?;
                        let last = section
                            .children
                            .iter()
                            .rev()
                            .find(|c| c.name == "on" || c.name == "off")?;
                        last.name == "on"
                    }
                };
                Input::Bool(value != *inverted)
            }
            Control::Number { .. } => Input::Number(node?.arg()?.as_f64()?),
            Control::Choice { .. } | Control::Text { .. } | Control::Color { .. } => return None,
        })
    }

    /// Whether the config ended up with what settings.kdl asks for.
    pub fn applied(&self, doc: &[Node], config: &Config) -> bool {
        match (self.wanted(doc), &self.control) {
            (Some(Input::Bool(want)), Control::Switch { get, inverted, .. }) => {
                (get(config) != *inverted) == want
            }
            (Some(Input::Number(want)), Control::Number { get, .. }) => {
                (get(config) - want).abs() < 1e-6
            }
            _ => true,
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
