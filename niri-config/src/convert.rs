//! Moving from niri: turns a niri `config.kdl` into bento's main config and the niri engine's
//! file, `engines/niri.kdl`.
//!
//! The conversion splices the original text, using the byte positions knuffel's parser gives every
//! node, so what bento keeps stays the way the person wrote it, comments and spacing included.
//! What moves to the engine's file takes the comments right above it along.
//!
//! - `layout {}` loses the settings that belong to the engine, which become `options {}` of the
//!   engine's file. The `layout {}` of an `output` or a `workspace` does the same, and its
//!   settings become the `options {}` of an `output` or `workspace` block of the engine's file.
//! - Window rules lose the properties that belong to the engine, which become `window-rule {}`
//!   blocks of the engine's file with the same matchers. That includes the colors and gradients
//!   of a `tab-indicator` rule.
//! - Binds to the engine's actions move to the engine's `binds {}`; the rest stay.
//!
//! Whatever can't be converted is dropped, and said in [`Conversion::notes`].

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::{fs, io};

use knuffel::ast::{self, Literal};
use knuffel::span::Span;

use crate::actions::native_actions;
use crate::tiling::{engine_config_path, TilingLayout};
use crate::{Color, GradientInterpolation};

type Node = ast::SpannedNode<Span>;

/// niri's layout actions, which the niri engine has under the same names.
const ENGINE_ACTIONS: &[&str] = &[
    "focus-column-left",
    "focus-column-right",
    "focus-column-first",
    "focus-column-last",
    "focus-column-right-or-first",
    "focus-column-left-or-last",
    "focus-column",
    "focus-column-or-monitor-left",
    "focus-column-or-monitor-right",
    "focus-window-in-column",
    "focus-window-down",
    "focus-window-up",
    "focus-window-or-monitor-down",
    "focus-window-or-monitor-up",
    "focus-window-or-workspace-down",
    "focus-window-or-workspace-up",
    "focus-window-down-or-column-left",
    "focus-window-down-or-column-right",
    "focus-window-up-or-column-left",
    "focus-window-up-or-column-right",
    "focus-window-top",
    "focus-window-bottom",
    "focus-window-down-or-top",
    "focus-window-up-or-bottom",
    "move-column-left",
    "move-column-right",
    "move-column-to-first",
    "move-column-to-last",
    "move-column-to-index",
    "move-column-left-or-to-monitor-left",
    "move-column-right-or-to-monitor-right",
    "move-column-to-workspace",
    "move-column-to-workspace-down",
    "move-column-to-workspace-up",
    "move-column-to-monitor",
    "move-column-to-monitor-left",
    "move-column-to-monitor-right",
    "move-column-to-monitor-up",
    "move-column-to-monitor-down",
    "move-column-to-monitor-previous",
    "move-column-to-monitor-next",
    "move-window-down",
    "move-window-up",
    "move-window-down-or-to-workspace-down",
    "move-window-up-or-to-workspace-up",
    "consume-or-expel-window-left",
    "consume-or-expel-window-right",
    "consume-window-into-column",
    "expel-window-from-column",
    "swap-window-left",
    "swap-window-right",
    "toggle-column-tabbed-display",
    "set-column-display",
    "center-column",
    "center-window",
    "center-visible-columns",
    "switch-preset-column-width",
    "switch-preset-column-width-back",
    "switch-preset-window-width",
    "switch-preset-window-width-back",
    "switch-preset-window-height",
    "switch-preset-window-height-back",
    "maximize-column",
    "set-column-width",
    "set-window-width",
    "set-window-height",
    "reset-window-height",
    "expand-column-to-available-width",
];

const TILING: &str = "tiling {\n    layout \"niri\"\n}\n\n";

const ENGINE_HEADER: &str = "\
// The niri engine's options, window rules and binds, converted from a niri config.
// https://github.com/faetalize/bento#layout-engines
";

/// A niri config, converted.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    /// bento's main config.
    pub config: String,
    /// The niri engine's file, `engines/niri.kdl` next to the main config.
    pub engine: String,
    /// What couldn't be converted, a sentence each.
    pub notes: Vec<String>,
}

/// Converts the text of a niri `config.kdl`.
pub fn convert(niri_config: &str) -> miette::Result<Conversion> {
    let document =
        knuffel::parse_ast::<Span>("config.kdl", niri_config).map_err(miette::Report::new)?;
    let mut converter = Converter {
        text: niri_config,
        native: native_actions().into_iter().map(|a| a.name).collect(),
        edits: Vec::new(),
        notes: Vec::new(),
        options: Vec::new(),
        scoped: Vec::new(),
        rules: Vec::new(),
        binds: Vec::new(),
        has_tiling: false,
    };
    converter.document(&document.nodes);
    Ok(converter.finish())
}

/// A piece of the original text to replace.
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

/// Something that moves to the engine's file, with the comments that describe it.
struct Moved {
    comments: Vec<String>,
    lines: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// One of bento's own actions.
    Native,
    /// One of the niri engine's.
    Engine,
    /// bento doesn't have it.
    Unknown,
}

/// The options that an `output` or `workspace` block of the engine's file sets.
struct Scoped {
    /// `output` or `workspace`.
    kind: &'static str,
    name: String,
    options: Vec<Moved>,
}

struct Converter<'a> {
    text: &'a str,
    native: HashSet<String>,
    edits: Vec<Edit>,
    notes: Vec<String>,
    options: Vec<Moved>,
    scoped: Vec<Scoped>,
    rules: Vec<String>,
    binds: Vec<Moved>,
    has_tiling: bool,
}

impl<'a> Converter<'a> {
    fn document(&mut self, nodes: &[Node]) {
        let mut includes = Vec::new();
        for node in nodes {
            match &**node.node_name {
                "layout" => self.layout(node, None),
                // The same section under an output or a workspace.
                "output" | "workspace" => {
                    let kind = if &**node.node_name == "output" {
                        "output"
                    } else {
                        "workspace"
                    };
                    let name = string_arg(node).unwrap_or_default();
                    for layout in node.children().filter(|c| &**c.node_name == "layout") {
                        self.layout(layout, Some((kind, name.clone())));
                    }
                }
                "window-rule" => self.window_rule(node),
                "binds" => self.binds(node),
                "tiling" => self.has_tiling = true,
                "include" => includes.extend(string_arg(node)),
                _ => (),
            }
        }

        if !includes.is_empty() {
            let files = includes
                .iter()
                .map(|f| format!("{f:?}"))
                .collect::<Vec<_>>();
            self.notes.push(format!(
                "The config includes {}. Included files aren't converted, so layout settings, \
                 window rules and binds in them need moving to engines/niri.kdl by hand.",
                files.join(", ")
            ));
        }
    }

    /// Moves the engine's settings out of a `layout` section, which is the top-level one or that
    /// of an output or a workspace, `scope` being its kind and name.
    fn layout(&mut self, node: &Node, scope: Option<(&'static str, String)>) {
        for child in node.children() {
            let mut notes = Vec::new();
            let Some(lines) = self.setting(child, &mut notes) else {
                continue;
            };
            let comments = self.comments(child);
            self.remove(child);
            self.notes.extend(notes);
            let moved = Moved { comments, lines };
            match &scope {
                None => self.options.push(moved),
                Some((kind, name)) => {
                    let index = self
                        .scoped
                        .iter()
                        .position(|scoped| scoped.kind == *kind && scoped.name == *name);
                    let scoped = match index {
                        Some(index) => &mut self.scoped[index],
                        None => {
                            self.scoped.push(Scoped {
                                kind,
                                name: name.clone(),
                                options: Vec::new(),
                            });
                            self.scoped.last_mut().unwrap()
                        }
                    };
                    scoped.options.push(moved);
                }
            }
        }
    }

    /// An engine setting of `layout` as option lines, or `None` if it isn't one. Settings with
    /// nothing to convert give no lines.
    fn setting(&self, node: &Node, notes: &mut Vec<String>) -> Option<Vec<String>> {
        let name = &**node.node_name;
        Some(match name {
            "preset-column-widths" | "preset-window-heights" => {
                let sizes: Vec<String> = node.children().filter_map(preset).collect();
                // niri takes an empty list to mean its defaults, which are the engine's too.
                if sizes.is_empty() {
                    Vec::new()
                } else {
                    vec![option(name, &sizes)]
                }
            }
            "default-column-width" => match node.children().next() {
                Some(first) => preset(first)
                    .map(|size| option(name, &[size]))
                    .into_iter()
                    .collect(),
                // Empty: the windows decide.
                None => vec![option(name, &[])],
            },
            "center-focused-column" | "default-column-display" => string_arg(node)
                .map(|value| option(name, &[quote(&value)]))
                .into_iter()
                .collect(),
            "always-center-single-column" => vec![option(name, &[flag(node).to_string()])],
            "tab-indicator" => self.tab_indicator(node, notes),
            "insert-hint" => self.insert_hint(node, notes),
            _ => return None,
        })
    }

    fn tab_indicator(&self, node: &Node, notes: &mut Vec<String>) -> Vec<String> {
        let mut lines = Vec::new();
        let (mut on, mut off) = (false, false);
        for child in node.children() {
            let name = &**child.node_name;
            let setting = format!("tab-indicator-{name}");
            match name {
                "on" => on = true,
                "off" => off = true,
                "hide-when-single-tab" | "place-within-column" => {
                    lines.push(option(&setting, &[flag(child).to_string()]));
                }
                "gap" | "width" | "corner-radius" | "gaps-between-tabs" => {
                    lines.extend(self.number(child).map(|n| option(&setting, &[n])));
                }
                "position" => {
                    lines.extend(string_arg(child).map(|v| option(&setting, &[quote(&v)])));
                }
                "length" => {
                    let property = child
                        .properties
                        .iter()
                        .find(|(k, _)| name_of(k) == "total-proportion");
                    match property.and_then(|(_, v)| number(&v.literal)) {
                        Some(value) => {
                            lines.push(option("tab-indicator-length", &[value.to_string()]));
                        }
                        None => notes.push(String::from(
                            "A tab-indicator length without total-proportion has no equivalent \
                             in niri.js, and was dropped.",
                        )),
                    }
                }
                "active-color" | "inactive-color" | "urgent-color" => match color_hex(child) {
                    Some(hex) => lines.push(option(&setting, &[quote(&hex)])),
                    None => notes.push(format!("The tab indicator's {name} couldn't be read.")),
                },
                "active-gradient" | "inactive-gradient" | "urgent-gradient" => {
                    match gradient_option(&setting, child) {
                        Some(line) => lines.push(line),
                        None => {
                            notes.push(format!("The tab indicator's {name} couldn't be read."));
                        }
                    }
                }
                _ => (),
            }
        }
        // `on` wins over `off`, wherever they are.
        if on {
            lines.insert(0, option("tab-indicator", &[String::from("true")]));
        } else if off {
            lines.insert(0, option("tab-indicator", &[String::from("false")]));
        }
        lines
    }

    fn insert_hint(&self, node: &Node, notes: &mut Vec<String>) -> Vec<String> {
        let mut lines = Vec::new();
        let (mut on, mut off) = (false, false);
        for child in node.children() {
            match &**child.node_name {
                "on" => on = true,
                "off" => off = true,
                "color" => match color_hex(child) {
                    Some(hex) => lines.push(option("insert-hint-color", &[quote(&hex)])),
                    None => notes.push(String::from("The insert hint's color couldn't be read.")),
                },
                "gradient" => match gradient_option("insert-hint-gradient", child) {
                    Some(line) => lines.push(line),
                    None => {
                        notes.push(String::from("The insert hint's gradient couldn't be read."))
                    }
                },
                _ => (),
            }
        }
        if on {
            lines.insert(0, option("insert-hint", &[String::from("true")]));
        } else if off {
            lines.insert(0, option("insert-hint", &[String::from("false")]));
        }
        lines
    }

    /// Moves the engine's properties out of a window rule.
    fn window_rule(&mut self, node: &Node) {
        let is_matcher = |c: &Node| matches!(&**c.node_name, "match" | "exclude");
        let matchers: Vec<&Node> = node.children().filter(|c| is_matcher(c)).collect();

        // The matcher can't be left out: without it the rule would reach windows it wasn't
        // written for, so none of the rule is kept.
        if matchers.iter().any(|m| {
            m.properties
                .keys()
                .any(|k| name_of(k) == "is-active-in-column")
        }) {
            let described: Vec<_> = matchers.iter().map(|m| self.source(m)).collect();
            self.notes.push(format!(
                "A window rule used is-active-in-column, which no engine has: {}. The whole \
                 rule was dropped.",
                described.join("; ")
            ));
            self.remove(node);
            return;
        }

        let mut moved: Vec<&Node> = Vec::new();
        let mut properties = Vec::new();
        for child in node.children() {
            let name = &**child.node_name;
            let lines: Vec<String> = match name {
                "default-column-width" | "default-window-height" => match child.children().next() {
                    Some(first) => preset(first)
                        .map(|size| option(name, &[size]))
                        .into_iter()
                        .collect(),
                    None => vec![option(name, &[])],
                },
                "default-column-display" => string_arg(child)
                    .map(|v| option(name, &[quote(&v)]))
                    .into_iter()
                    .collect(),
                "open-maximized" => vec![option(name, &[flag(child).to_string()])],
                "tab-indicator" => self.tab_indicator_rule(child),
                _ => continue,
            };
            moved.push(child);
            properties.push(Moved {
                comments: self.comments(child),
                lines,
            });
        }
        if moved.is_empty() {
            return;
        }

        let whole = node
            .children()
            .all(|c| is_matcher(c) || moved.iter().any(|m| std::ptr::eq(*m, c)));
        if whole {
            // Nothing is left of the rule here, so its description goes with it.
            let comments = self.comments(node);
            self.remove(node);
            self.rule_block(&comments, &matchers, &properties);
        } else {
            for child in &moved {
                self.remove(child);
            }
            self.rule_block(&[], &matchers, &properties);
        }
    }

    /// The colors and gradients that a window rule's `tab-indicator` sets for the window's tab, as
    /// the engine's window rule properties.
    fn tab_indicator_rule(&mut self, node: &Node) -> Vec<String> {
        let mut lines = Vec::new();
        for child in node.children() {
            let name = &**child.node_name;
            let setting = format!("tab-indicator-{name}");
            let line = match name {
                "active-color" | "inactive-color" | "urgent-color" => {
                    color_hex(child).map(|hex| option(&setting, &[quote(&hex)]))
                }
                "active-gradient" | "inactive-gradient" | "urgent-gradient" => {
                    gradient_option(&setting, child)
                }
                _ => continue,
            };
            match line {
                Some(line) => lines.push(line),
                None => self.notes.push(format!(
                    "A window rule's tab-indicator {name} couldn't be read, and was dropped."
                )),
            }
        }
        lines
    }

    fn rule_block(&mut self, comments: &[String], matchers: &[&Node], properties: &[Moved]) {
        if properties.iter().all(|property| property.lines.is_empty()) {
            return;
        }
        let mut block = String::new();
        for comment in comments {
            let _ = writeln!(block, "{comment}");
        }
        block.push_str("window-rule {\n");
        for matcher in matchers {
            let _ = writeln!(block, "    {}", self.source(matcher));
        }
        if !matchers.is_empty() {
            block.push('\n');
        }
        block.push_str(&render(properties));
        block.push_str("\n}");
        self.rules.push(block);
    }

    /// Moves binds of the engine's actions to its file, and drops the ones bento can't keep.
    fn binds(&mut self, node: &Node) {
        for bind in node.children() {
            let actions = bind_actions(bind);
            let kinds: Vec<Kind> = actions.iter().map(|a| self.kind(a)).collect();
            if kinds.iter().all(|k| *k == Kind::Native) {
                continue;
            }

            let key = &**bind.node_name;
            if kinds.iter().all(|k| *k == Kind::Engine) {
                let text = self.engine_bind(bind, &actions);
                let comments = self.comments(bind);
                self.binds.push(Moved {
                    comments,
                    lines: vec![text],
                });
            } else if let Some(unknown) = actions
                .iter()
                .zip(&kinds)
                .find_map(|(a, k)| (*k == Kind::Unknown).then_some(a))
            {
                self.notes.push(format!(
                    "bento has no {} action, so the bind {key} was dropped.",
                    &**unknown.node_name
                ));
            } else {
                self.notes.push(format!(
                    "The bind {key} runs both an action of the niri engine and one of bento's, \
                     which no single file can hold, so it was dropped.",
                ));
            }
            self.remove(bind);
        }
    }

    fn kind(&self, action: &Node) -> Kind {
        let name = &**action.node_name;
        if self.native.contains(name) {
            Kind::Native
        } else if ENGINE_ACTIONS.contains(&name) {
            Kind::Engine
        } else {
            Kind::Unknown
        }
    }

    /// The text of a bind to an engine's action. Engine actions take their arguments by
    /// position, so niri's properties, like `focus=false`, become the last arguments.
    fn engine_bind(&self, bind: &Node, actions: &[&Node]) -> String {
        let start = bind.node_name.span().0;
        let mut text = self.text[start..bind.span().1].to_owned();

        let mut heads = Vec::new();
        for action in actions.iter().filter(|a| !a.properties.is_empty()) {
            let mut end = action.node_name.span().1;
            let mut head = String::from(&**action.node_name);
            for arg in &action.arguments {
                end = end.max(arg.literal.span().1);
                head.push(' ');
                head.push_str(self.slice(arg.literal.span()));
            }
            for value in action.properties.values() {
                end = end.max(value.literal.span().1);
                head.push(' ');
                head.push_str(self.slice(value.literal.span()));
            }
            heads.push((action.node_name.span().0, end, head));
        }
        heads.sort_by_key(|(start, ..)| std::cmp::Reverse(*start));
        for (head_start, head_end, head) in heads {
            text.replace_range(head_start - start..head_end - start, &head);
        }
        text.trim_end().to_owned()
    }

    fn finish(mut self) -> Conversion {
        if self.has_tiling {
            self.notes.push(String::from(
                "The config already has a tiling section. Set its layout to \"niri\".",
            ));
        } else {
            self.edits.push(Edit {
                start: 0,
                end: 0,
                text: String::from(TILING),
            });
        }

        // From the end, so earlier positions stay where they are.
        self.edits
            .sort_by_key(|e| std::cmp::Reverse((e.start, e.end)));
        let mut config = self.text.to_owned();
        for edit in &self.edits {
            config.replace_range(edit.start..edit.end, &edit.text);
        }

        let mut engine = String::from(ENGINE_HEADER);
        if !self.options.is_empty() {
            let options = render(&self.options);
            let _ = write!(engine, "\noptions {{\n{options}\n}}\n");
        }
        for scoped in &self.scoped {
            let options = render(&scoped.options);
            if options.is_empty() {
                continue;
            }
            // Two levels down: the block, and its `options`.
            let nest = |line: &str| match line.is_empty() {
                true => String::new(),
                false => format!("    {line}"),
            };
            let options: Vec<String> = options.lines().map(nest).collect();
            let name = quote(&scoped.name);
            let kind = scoped.kind;
            let options = options.join("\n");
            let _ = write!(
                engine,
                "\n{kind} {name} {{\n    options {{\n{options}\n    }}\n}}\n"
            );
        }
        for rule in &self.rules {
            let _ = write!(engine, "\n{rule}\n");
        }
        if !self.binds.is_empty() {
            let binds = render(&self.binds);
            let _ = write!(engine, "\nbinds {{\n{binds}\n}}\n");
        }

        let mut notes = Vec::new();
        for note in self.notes {
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        Conversion {
            config,
            engine,
            notes,
        }
    }

    // Text positions.

    fn slice(&self, span: &Span) -> &'a str {
        &self.text[span.0..span.1]
    }

    /// A node as written, without its terminator.
    fn source(&self, node: &Node) -> &'a str {
        let text = &self.text[node.node_name.span().0..node.span().1];
        text.trim_end().trim_end_matches(';').trim_end()
    }

    fn number(&self, node: &Node) -> Option<String> {
        let arg = node.arguments.first()?;
        number(&arg.literal)?;
        Some(self.slice(arg.literal.span()).to_owned())
    }

    /// Where the `//` comment lines right above a node begin, if it's alone on its line: they
    /// describe it. Otherwise where the node begins.
    fn comment_start(&self, node: &Node) -> usize {
        let start = node.node_name.span().0;
        let line = line_start(self.text, start);
        if !self.text[line..start].trim().is_empty() {
            return start;
        }
        let mut top = line;
        while top > 0 {
            let above = line_start(self.text, top - 1);
            if !self.text[above..top].trim_start().starts_with("//") {
                break;
            }
            top = above;
        }
        top
    }

    /// The comment lines that describe a node, without their indentation.
    fn comments(&self, node: &Node) -> Vec<String> {
        let top = self.comment_start(node);
        let line = line_start(self.text, node.node_name.span().0);
        let comments = &self.text[top..line];
        comments.lines().map(|l| l.trim().to_owned()).collect()
    }

    /// Removes a node, and the comments that describe it. A node alone on its lines takes them
    /// all; one that shares its line only takes itself and the spaces after it.
    fn remove(&mut self, node: &Node) {
        let start = node.node_name.span().0;
        let end = node.span().1;
        let line = line_start(self.text, start);
        let alone_before = self.text[line..start].trim().is_empty();
        // The node ends at its newline, or at a `;` with more of the line after it.
        let ends_line = self.text[..end].ends_with('\n');
        let rest = if ends_line {
            ""
        } else {
            self.text[end..].split('\n').next().unwrap_or_default()
        };

        let (start, end) = if alone_before && rest.trim().is_empty() {
            let mut end = end + rest.len();
            if !ends_line && self.text[end..].starts_with('\n') {
                end += 1;
            }
            let start = self.comment_start(node);
            // Don't leave two blank lines where there was one.
            if self.text[..start].ends_with("\n\n") && self.text[end..].starts_with('\n') {
                end += 1;
            }
            (start, end)
        } else {
            let after = &self.text[end..];
            let spaces = after.len() - after.trim_start_matches(' ').len();
            (start, end + spaces)
        };
        self.edits.push(Edit {
            start,
            end,
            text: String::new(),
        });
    }
}

/// The action nodes of a bind: its own, or the ones in its `press` and `release` sections.
fn bind_actions(bind: &Node) -> Vec<&Node> {
    let mut actions = Vec::new();
    for child in bind.children() {
        if matches!(&**child.node_name, "press" | "release") {
            actions.extend(child.children());
        } else {
            actions.push(child);
        }
    }
    actions
}

/// Renders what moves to a section of the engine's file: comments and lines, indented to sit in
/// the section, with a blank line before commented ones.
fn render(moved: &[Moved]) -> String {
    let mut out = String::new();
    for moved in moved {
        if moved.lines.is_empty() {
            continue;
        }
        if !out.is_empty() {
            if !moved.comments.is_empty() {
                out.push('\n');
            }
            out.push('\n');
        }
        let comments = moved.comments.iter();
        for line in comments.chain(&moved.lines) {
            let _ = write!(out, "    {line}");
            out.push('\n');
        }
        out.pop();
    }
    out
}

/// A setting as a line of the engine's `options {}`: `name value...`.
fn option(name: &str, values: &[String]) -> String {
    let mut line = String::from(name);
    for value in values {
        line.push(' ');
        line.push_str(value);
    }
    line
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn name_of(name: &ast::SpannedName<Span>) -> &str {
    name
}

fn string_arg(node: &Node) -> Option<String> {
    match &*node.arguments.first()?.literal {
        Literal::String(value) => Some(value.to_string()),
        _ => None,
    }
}

/// A flag like `always-center-single-column`: on by itself, or with an explicit value.
fn flag(node: &Node) -> bool {
    match node.arguments.first().map(|arg| &*arg.literal) {
        Some(Literal::Bool(value)) => *value,
        _ => true,
    }
}

fn number(literal: &Literal) -> Option<f64> {
    match literal {
        Literal::Int(value) => i64::try_from(value).ok().map(|n| n as f64),
        Literal::Decimal(value) => f64::try_from(value).ok(),
        _ => None,
    }
}

/// A `proportion 0.5` or `fixed 960` child, as the size the engine's options take.
fn preset(node: &Node) -> Option<String> {
    let value = number(&node.arguments.first()?.literal)?;
    match &**node.node_name {
        "proportion" => {
            // Rounded, so 0.33333 reads "33.333%" instead of "33.333000000000006%".
            let percent = (value * 100. * 1e6).round() / 1e6;
            Some(format!("\"{percent}%\""))
        }
        "fixed" => Some(format!("\"{value}\"")),
        _ => None,
    }
}

/// A niri gradient node, like `active-gradient from="red" to="#00f" angle=45`, as the engine's
/// option or window rule property `setting`, with the colors as `#rrggbb` and niri's
/// `relative-to="workspace-view"` as `"view"`. `None` if it can't be read.
fn gradient_option(setting: &str, node: &Node) -> Option<String> {
    let (mut from, mut to, mut angle, mut relative_to, mut space) = (None, None, None, None, None);
    for (key, value) in &node.properties {
        match (name_of(key), &*value.literal) {
            ("from", Literal::String(text)) => from = Color::from_str(text).ok(),
            ("to", Literal::String(text)) => to = Color::from_str(text).ok(),
            ("angle", literal) => {
                angle = Some(number(literal).filter(|angle| angle.fract() == 0.)?);
            }
            ("relative-to", Literal::String(text)) => {
                relative_to = match &**text {
                    "window" => None,
                    "workspace-view" => Some("view"),
                    _ => return None,
                };
            }
            ("in", Literal::String(text)) => {
                text.parse::<GradientInterpolation>().ok()?;
                space = Some(text.to_string());
            }
            _ => return None,
        }
    }

    let mut line = option(setting, &[]);
    let _ = write!(line, " from={} to={}", quote(&hex(from?)), quote(&hex(to?)));
    if let Some(angle) = angle {
        let _ = write!(line, " angle={angle}");
    }
    if let Some(relative_to) = relative_to {
        let _ = write!(line, " relative-to={}", quote(relative_to));
    }
    if let Some(space) = space {
        let _ = write!(line, " in={}", quote(&space));
    }
    Some(line)
}

/// A color as the `#rrggbb` or `#rrggbbaa` the engine's options take.
fn hex(color: Color) -> String {
    let byte = |x: f32| (x.clamp(0., 1.) * 255.).round() as u8;
    let mut hex = format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.r),
        byte(color.g),
        byte(color.b)
    );
    if byte(color.a) != 255 {
        let _ = write!(hex, "{:02x}", byte(color.a));
    }
    hex
}

/// A niri color node, which is a CSS color string or four 0-255 numbers, as the `#rrggbb` or
/// `#rrggbbaa` the engine's options take.
fn color_hex(node: &Node) -> Option<String> {
    let first = node.arguments.first()?;
    let color = match &*first.literal {
        Literal::String(value) => Color::from_str(value).ok()?,
        _ => {
            let numbers = node.arguments.iter().map(|a| number(&a.literal));
            let bytes: Option<Vec<u8>> = numbers.map(|n| u8::try_from(n? as i64).ok()).collect();
            let bytes = bytes?;
            let &[r, g, b, a] = bytes.as_slice() else {
                return None;
            };
            Color::from_rgba8_unpremul(r, g, b, a)
        }
    };
    Some(hex(color))
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// What a migration wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    /// Where the existing files went before they were replaced.
    pub backups: Vec<PathBuf>,
    pub config: PathBuf,
    pub engine: PathBuf,
}

/// Writes a conversion as bento's main config at `config_path` and the niri engine's file next to
/// it. Files that exist are kept as backups first, with `.before-migration` added to their names.
pub fn write(conversion: &Conversion, config_path: &Path) -> io::Result<Written> {
    let layout = TilingLayout::BuiltIn(String::from("niri"));
    let engine_path = engine_config_path(config_path, &layout);

    let mut backups = Vec::new();
    for path in [config_path, engine_path.as_path()] {
        if path.exists() {
            let backup = backup_path(path);
            fs::copy(path, &backup)?;
            backups.push(backup);
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
    }

    // The engine's file first, so the main config only picks niri once it's there.
    fs::write(&engine_path, &conversion.engine)?;
    fs::write(config_path, &conversion.config)?;
    Ok(Written {
        backups,
        config: config_path.to_owned(),
        engine: engine_path,
    })
}

/// `path` with `.before-migration` added, or a number after it if that file exists, so an older
/// backup is never overwritten.
fn backup_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut backup = path.with_file_name(format!("{name}.before-migration"));
    let mut n = 1;
    while backup.exists() {
        n += 1;
        backup = path.with_file_name(format!("{name}.before-migration-{n}"));
    }
    backup
}

#[cfg(test)]
mod tests {
    use niri_ipc::LayoutEngineGradient;

    use super::*;
    use crate::{Action, Config, Key, TilingOption};

    /// A niri config like the default one that niri writes, with the settings of an engine
    /// changed.
    const NIRI: &str = r##"
// This config is in the KDL format: https://kdl.dev
input {
    keyboard {
        xkb {
            layout "us"
        }
    }
    touchpad {
        tap
    }
}

// Settings that influence how windows are positioned and sized.
layout {
    // Set gaps around windows in logical pixels.
    gaps 16

    // When to center a column when changing focus.
    center-focused-column "on-overflow"

    // You can customize the widths that "switch-preset-column-width" cycles through.
    preset-column-widths {
        proportion 0.33333
        proportion 0.5
        fixed 1920
    }

    default-column-width { proportion 0.5; }

    focus-ring {
        width 4
        active-color "#7fc8ff"
    }

    tab-indicator {
        width 6
        position "top"
        length total-proportion=0.75
        active-color 10 20 30 255
        inactive-color "rgb(255, 0, 0)"
        gaps-between-tabs 2
        active-gradient from="#80c8ff" to="red" angle=45 relative-to="workspace-view" in="oklch shorter hue"
        urgent-gradient from="#900" to="#f00"
    }

    insert-hint {
        color "#ff000080"
        gradient from="#ff000080" to="#0000ff80" relative-to="window"
    }
}

spawn-at-startup "waybar"

prefer-no-csd

window-rule {
    match app-id=r#"^org\.wezfurlong\.wezterm$"#
    default-column-width {}
}

// Open the Firefox picture-in-picture player as floating by default.
window-rule {
    match app-id=r#"firefox$"# title="^Picture-in-Picture$"
    open-floating true
}

window-rule {
    match app-id="mpv"
    open-maximized true
    default-window-height { fixed 500; }
    opacity 0.9
}

// Kitty's tab stands out.
window-rule {
    match app-id="kitty"
    tab-indicator {
        active-color "#ff0000"
        inactive-gradient from="#111" to="#222" angle=90
    }
}

window-rule {
    match is-active-in-column=false
    opacity 0.8
}

binds {
    // Mod-Shift-/ shows a list of important hotkeys.
    Mod+Shift+Slash { show-hotkey-overlay; }

    Mod+T hotkey-overlay-title="Open a Terminal: alacritty" { spawn "alacritty"; }

    Mod+Q { close-window; }

    // Focus the column to the left.
    Mod+Left  { focus-column-left; }
    Mod+Right { focus-column-right; }
    Mod+Ctrl+Left { move-column-left; }

    Mod+Shift+Left { move-column-to-monitor-left; }
    Mod+Shift+Down { move-column-to-workspace-down focus=false; }
    Mod+Shift+Up { move-column-to-workspace 2 focus=false; }
    Mod+R { switch-preset-column-width; }
    Mod+Minus { set-column-width "-10%"; }

    Mod+1 { focus-workspace 1; }
    Mod+Shift+E { quit; }
    Mod+Shift+U { toggle-window-urgent; }
    Mod+WheelScrollDown cooldown-ms=150 { focus-workspace-down; }
}

output "eDP-1" {
    scale 2
    layout {
        default-column-width { proportion 0.25; }
        gaps 8
        tab-indicator {
            width 3
            active-gradient from="#000" to="#fff"
        }
    }
}

workspace "web" {
    layout {
        // Tabs on this one.
        default-column-display "tabbed"
    }
}
"##;

    fn convert_niri() -> (Conversion, Config) {
        let converted = convert(NIRI).unwrap();
        let config = Config::parse_mem_with_engine(&converted.config, &converted.engine)
            .unwrap_or_else(|err| {
                let err = miette::Report::new(err);
                panic!(
                    "{err:?}\n--- config.kdl\n{}\n--- niri.kdl\n{}",
                    converted.config, converted.engine
                )
            });
        (converted, config)
    }

    fn strings(values: &[&str]) -> Vec<TilingOption> {
        values
            .iter()
            .map(|v| TilingOption::String(String::from(*v)))
            .collect()
    }

    #[test]
    fn the_result_is_bentos_config_for_the_niri_engine() {
        let (converted, config) = convert_niri();

        assert_eq!(
            config.tiling.layout,
            TilingLayout::BuiltIn(String::from("niri"))
        );
        assert!(converted
            .config
            .starts_with("tiling {\n    layout \"niri\"\n}\n"));

        // What bento keeps is as it was written.
        assert_eq!(config.layout.gaps, 16.);
        assert!(converted
            .config
            .contains("// Set gaps around windows in logical pixels."));
        assert!(converted.config.contains("focus-ring {\n        width 4"));
        assert!(converted
            .config
            .contains("// Open the Firefox picture-in-picture player"));
        assert!(config.prefer_no_csd);
        assert_eq!(config.spawn_at_startup.len(), 1);
    }

    #[test]
    fn layout_settings_become_engine_options() {
        let (converted, config) = convert_niri();
        let option = |name: &str| {
            let option = config.tiling.options.iter().find(|(n, _)| n == name);
            option.map(|(_, values)| values.clone())
        };

        assert_eq!(
            option("center-focused-column"),
            Some(strings(&["on-overflow"]))
        );
        assert_eq!(
            option("preset-column-widths"),
            Some(strings(&["33.333%", "50%", "1920"]))
        );
        assert_eq!(option("default-column-width"), Some(strings(&["50%"])));
        assert_eq!(
            option("tab-indicator-width"),
            Some(vec![TilingOption::Number(6.)])
        );
        assert_eq!(option("tab-indicator-position"), Some(strings(&["top"])));
        assert_eq!(
            option("tab-indicator-length"),
            Some(vec![TilingOption::Number(0.75)])
        );
        assert_eq!(
            option("tab-indicator-active-color"),
            Some(strings(&["#0a141e"]))
        );
        assert_eq!(
            option("tab-indicator-inactive-color"),
            Some(strings(&["#ff0000"]))
        );
        assert_eq!(option("insert-hint-color"), Some(strings(&["#ff000080"])));
        assert_eq!(
            option("tab-indicator-gaps-between-tabs"),
            Some(vec![TilingOption::Number(2.)])
        );

        // Gradients are written with properties, in the engine's spelling.
        let gradient = |from: &str, to: &str| LayoutEngineGradient {
            from: String::from(from),
            to: String::from(to),
            angle: None,
            relative_to: None,
            color_space: None,
        };
        assert_eq!(
            option("tab-indicator-active-gradient"),
            Some(vec![TilingOption::Gradient(LayoutEngineGradient {
                angle: Some(45.),
                relative_to: Some(String::from("view")),
                color_space: Some(String::from("oklch shorter hue")),
                ..gradient("#80c8ff", "#ff0000")
            })])
        );
        assert_eq!(
            option("tab-indicator-urgent-gradient"),
            Some(vec![TilingOption::Gradient(gradient("#990000", "#ff0000"))])
        );
        // niri's default, `relative-to="window"`, is the engine's too.
        assert_eq!(
            option("insert-hint-gradient"),
            Some(vec![TilingOption::Gradient(gradient(
                "#ff000080",
                "#0000ff80"
            ))])
        );

        // None of it is left in bento's config, but gaps are.
        for name in [
            "center-focused-column",
            "preset-column-widths",
            "tab-indicator",
            "insert-hint",
        ] {
            assert!(!converted.config.contains(name), "{name}");
        }
        // The comments that described them moved with them.
        assert!(converted
            .engine
            .contains("    // When to center a column when changing focus."));
        assert!(!converted.config.contains("When to center a column"));
    }

    #[test]
    fn window_rule_properties_move_with_their_matchers() {
        let (converted, config) = convert_niri();

        // The rules that only set the engine's properties are gone from the main config; the
        // mpv one keeps what bento has.
        assert_eq!(config.window_rules.len(), 2);
        assert_eq!(config.window_rules[1].opacity, Some(0.9));
        assert!(!converted.config.contains("wezterm"));
        assert!(!converted.config.contains("open-maximized"));

        let rules = &config.tiling.window_rules;
        assert_eq!(rules.len(), 3);

        let wezterm = &rules[0];
        assert_eq!(wezterm.matches.len(), 1);
        assert_eq!(
            wezterm.properties.get("default-column-width"),
            Some(&[][..])
        );

        let mpv = &rules[1];
        assert_eq!(mpv.matches.len(), 1);
        let open = [TilingOption::Bool(true)];
        assert_eq!(mpv.properties.get("open-maximized"), Some(&open[..]));
        let height = strings(&["500"]);
        assert_eq!(
            mpv.properties.get("default-window-height"),
            Some(&height[..])
        );
    }

    #[test]
    fn a_tab_indicator_rule_sets_the_tab_colors_of_its_windows() {
        let (converted, config) = convert_niri();
        assert!(!converted.config.contains("kitty"));
        assert!(!converted.config.contains("Kitty's tab"));
        assert!(converted.engine.contains("// Kitty's tab stands out."));

        let kitty = &config.tiling.window_rules[2];
        assert_eq!(kitty.matches.len(), 1);
        assert_eq!(
            kitty.properties.get("tab-indicator-active-color"),
            Some(&strings(&["#ff0000"])[..])
        );
        let Some([TilingOption::Gradient(gradient)]) =
            kitty.properties.get("tab-indicator-inactive-gradient")
        else {
            panic!("{:?}", kitty.properties);
        };
        assert_eq!(
            (gradient.from.as_str(), gradient.to.as_str()),
            ("#111111", "#222222")
        );
        assert_eq!(gradient.angle, Some(90.));
    }

    #[test]
    fn output_and_workspace_layouts_become_scoped_options() {
        let (converted, config) = convert_niri();
        let tiling = &config.tiling;

        assert_eq!(tiling.output_options.len(), 1);
        let output = &tiling.output_options[0];
        assert_eq!(output.name, "eDP-1");
        let names: Vec<_> = output.options.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "default-column-width",
                "tab-indicator-width",
                "tab-indicator-active-gradient"
            ]
        );
        assert_eq!(output.options[0].1, strings(&["25%"]));

        assert_eq!(tiling.workspace_options.len(), 1);
        let workspace = &tiling.workspace_options[0];
        assert_eq!(workspace.name, "web");
        assert_eq!(
            workspace.options,
            [(String::from("default-column-display"), strings(&["tabbed"]))]
        );
        // The comment went along.
        assert!(converted
            .engine
            .contains("        // Tabs on this one.\n        default-column-display"));

        // What isn't the engine's stays where it was.
        assert!(converted.config.contains("scale 2"));
        assert!(converted.config.contains("gaps 8"));
        assert!(!converted.config.contains("0.25"));
        assert!(!converted.config.contains("tabbed"));

        // The result is a config bento reads, so the options reach the right workspaces.
        let output_name = crate::OutputName {
            connector: String::from("edp-1"),
            make: None,
            model: None,
            serial: None,
        };
        let layers = tiling.option_layers(Some(&output_name), Some("web"));
        assert_eq!(layers.len(), 3);
    }

    #[test]
    fn rules_with_is_active_in_column_are_dropped() {
        let (converted, config) = convert_niri();
        assert!(!converted.config.contains("is-active-in-column"));
        assert!(!converted.config.contains("opacity 0.8"));
        assert_eq!(config.window_rules.len(), 2);
        assert!(converted
            .notes
            .iter()
            .any(|note| note.contains("is-active-in-column") && note.contains("whole rule")));
    }

    #[test]
    fn binds_go_to_the_file_of_their_action() {
        let (converted, config) = convert_niri();

        let action = |key: &str| {
            let key: Key = key.parse().unwrap();
            let bind = config.binds.0.iter().find(|bind| bind.key == key);
            bind.and_then(|bind| bind.press_action()).cloned()
        };
        let engine = |name: &str, args: Vec<TilingOption>| {
            Some(Action::LayoutAction(String::from(name), args, None))
        };

        assert_eq!(action("Mod+Left"), engine("focus-column-left", vec![]));
        assert_eq!(
            action("Mod+Minus"),
            engine("set-column-width", strings(&["-10%"]))
        );
        assert_eq!(
            action("Mod+Shift+Left"),
            engine("move-column-to-monitor-left", vec![])
        );
        // Engine actions take their arguments by position.
        assert_eq!(
            action("Mod+Shift+Down"),
            engine(
                "move-column-to-workspace-down",
                vec![TilingOption::Bool(false)]
            )
        );
        assert_eq!(
            action("Mod+Shift+Up"),
            engine(
                "move-column-to-workspace",
                vec![TilingOption::Number(2.), TilingOption::Bool(false)]
            )
        );

        // bento's own binds stay in its config, as written.
        assert_eq!(action("Mod+Q"), Some(Action::CloseWindow));
        assert!(converted.config.contains("Mod+Q { close-window; }"));
        assert!(converted
            .config
            .contains("Mod+T hotkey-overlay-title=\"Open a Terminal: alacritty\""));
        assert!(converted
            .config
            .contains("// Mod-Shift-/ shows a list of important hotkeys."));
        assert!(converted
            .config
            .contains("Mod+WheelScrollDown cooldown-ms=150"));
        assert!(!converted.config.contains("focus-column"));

        // The comment that described a moved bind went with it.
        assert!(converted
            .engine
            .contains("    // Focus the column to the left.\n    Mod+Left"));
        assert!(!converted.config.contains("Focus the column to the left"));
    }

    #[test]
    fn what_can_not_be_converted_is_dropped_with_a_note() {
        let (converted, config) = convert_niri();

        // niri's urgent actions don't exist in bento.
        let urgent: Key = "Mod+Shift+U".parse().unwrap();
        assert!(config.binds.0.iter().all(|bind| bind.key != urgent));
        let has_note = |text: &str| converted.notes.iter().any(|note| note.contains(text));
        assert!(has_note("bento has no toggle-window-urgent action"));
        assert!(has_note("Mod+Shift+U"));
        // Everything of the tab indicator and the insert hint can be converted.
        assert!(!has_note("tab-indicator"));
        assert!(!has_note("insert-hint"));
    }

    #[test]
    fn gradients_that_can_not_be_read_are_dropped_with_a_note() {
        let niri = r##"
layout {
    tab-indicator {
        active-gradient from="not a color" to="#fff"
        inactive-gradient from="#000" to="#fff" in="hsl"
        urgent-gradient from="#000" to="#fff" relative-to="somewhere"
    }
    insert-hint {
        gradient from="#000"
    }
}
window-rule {
    match app-id="x"
    tab-indicator {
        active-gradient from="#000" to="#fff" angle=1.5
    }
}
"##;
        let converted = convert(niri).unwrap();
        assert!(
            !converted.engine.contains("gradient"),
            "{}",
            converted.engine
        );
        let has_note = |text: &str| converted.notes.iter().any(|note| note.contains(text));
        assert!(has_note("tab indicator's active-gradient couldn't be read"));
        assert!(has_note(
            "tab indicator's inactive-gradient couldn't be read"
        ));
        assert!(has_note("tab indicator's urgent-gradient couldn't be read"));
        assert!(has_note("insert hint's gradient couldn't be read"));
        assert!(has_note("tab-indicator active-gradient couldn't be read"));
    }

    #[test]
    fn includes_are_left_to_the_person() {
        let converted = convert("include \"colors.kdl\"\nlayout { gaps 4; }\n").unwrap();
        assert!(converted.config.contains("include \"colors.kdl\""));
        assert!(converted.notes.iter().any(|n| n.contains("colors.kdl")));
    }

    #[test]
    fn a_config_without_engine_settings_only_picks_the_engine() {
        let converted = convert("layout {\n    gaps 4\n}\n").unwrap();
        assert_eq!(
            converted.config,
            "tiling {\n    layout \"niri\"\n}\n\nlayout {\n    gaps 4\n}\n"
        );
        assert_eq!(converted.engine, ENGINE_HEADER);
        assert!(converted.notes.is_empty());
    }

    #[test]
    fn invalid_kdl_is_an_error() {
        assert!(convert("layout {").is_err());
    }

    #[test]
    fn settings_written_as_the_defaults_are_still_options() {
        let niri = r#"
layout {
    center-focused-column "never"
    always-center-single-column
    preset-column-widths {
        proportion 0.33333
        proportion 0.5
        proportion 0.66667
    }
    insert-hint { off; }
}
"#;
        let converted = convert(niri).unwrap();
        let config = Config::parse_mem_with_engine(&converted.config, &converted.engine).unwrap();
        let names: Vec<_> = config
            .tiling
            .options
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "center-focused-column",
                "always-center-single-column",
                "preset-column-widths",
                "insert-hint"
            ]
        );
        assert_eq!(config.tiling.options[1].1, vec![TilingOption::Bool(true)]);
        assert_eq!(config.tiling.options[3].1, vec![TilingOption::Bool(false)]);
    }

    #[test]
    fn writing_keeps_what_was_there() {
        let dir = std::env::temp_dir().join(format!("bento-migration-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_path = dir.join("config.kdl");
        let engine_path = dir.join("engines/niri.kdl");

        let converted = convert("layout { gaps 4; }\n").unwrap();

        // Nothing there yet: nothing to back up.
        let written = write(&converted, &config_path).unwrap();
        assert!(written.backups.is_empty());
        assert_eq!(fs::read_to_string(&config_path).unwrap(), converted.config);
        assert_eq!(fs::read_to_string(&engine_path).unwrap(), converted.engine);

        // The second time, the first result is kept, and a third keeps both.
        fs::write(&config_path, "mine").unwrap();
        write(&converted, &config_path).unwrap();
        fs::write(&config_path, "mine again").unwrap();
        let written = write(&converted, &config_path).unwrap();

        let backup = |name: &str| fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(backup("config.kdl.before-migration"), "mine");
        assert_eq!(backup("config.kdl.before-migration-2"), "mine again");
        assert_eq!(written.backups.len(), 2);
        assert!(dir.join("engines/niri.kdl.before-migration").exists());

        fs::remove_dir_all(&dir).unwrap();
    }
}
