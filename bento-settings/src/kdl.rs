//! Edits to a hand-written KDL config that leave everything else in it alone.
//!
//! knuffel parses the text with byte positions for every node and value. A change replaces only
//! the node it's about, or adds one line to the section it belongs in at the section's own
//! indentation, so comments, ordering and spacing stay the way the person wrote them.
//!
//! Paths like `input.touchpad.tap` find the node bento actually uses: where a section appears more
//! than once, the last one that has the setting wins, since later sections override earlier ones.
//! Sections that bento replaces as a whole only count in their last copy, and window rules only
//! count when they apply to every window.

use std::fmt::Write as _;

use knuffel::ast;
use knuffel::span::Span;
use niri_config::Key;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// What follows a node's name: its arguments, then its properties.
#[derive(Clone, Copy)]
struct Values<'a> {
    args: &'a [Value],
    properties: &'a [(String, Value)],
}

impl Values<'_> {
    /// A node that is only a section.
    const NONE: Values<'static> = Values {
        args: &[],
        properties: &[],
    };
}

/// Sections that a later copy replaces as a whole, instead of merging into. Only the last copy of
/// one of these is what bento uses.
const WHOLE_SECTIONS: &[&str] = &[
    "tiling",
    "input.keyboard.xkb",
    "input.touchpad",
    "input.mouse",
    "input.trackpoint",
    "input.trackball",
    "input.tablet",
    "input.touch",
    "layout.struts",
];

/// Sections that hold rules, where only rules without `match` or `exclude` apply to every window.
const RULES: &[&str] = &["window-rule", "layer-rule"];

pub fn split(path: &str) -> Vec<&str> {
    path.split('.').collect()
}

/// A node with where its parts sit in the text.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    /// Start of the node's name.
    start: usize,
    /// End of the node's name, arguments and properties, before any `{`.
    head_end: usize,
    /// End of the node, after its terminator.
    end: usize,
    /// Positions of the `{` and `}` around its children.
    braces: Option<(usize, usize)>,
    pub children: Vec<Node>,
}

pub struct Document {
    text: String,
}

impl Document {
    /// Checks the text parses, so later edits can rely on it.
    pub fn parse(text: String) -> Result<Self, knuffel::Error> {
        knuffel::parse_ast::<Span>("config.kdl", &text)?;
        Ok(Self { text })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn nodes(&self) -> Vec<Node> {
        let doc = knuffel::parse_ast::<Span>("config.kdl", &self.text)
            .expect("edits keep the document valid");
        doc.nodes
            .iter()
            .map(|node| locate(node, &self.text))
            .collect()
    }

    /// The node bento uses at this path, if the document sets it.
    pub fn get(&self, path: &[&str]) -> Option<Node> {
        find(&self.nodes(), path, "")
            .last()
            .map(|node| (*node).clone())
    }

    /// Sets the arguments of the node at `path`, adding it and its sections if they're missing.
    pub fn set(&mut self, path: &[&str], args: Vec<Value>) {
        self.set_node(path, args, Vec::new());
    }

    /// Sets the arguments and properties of the node at `path`, like a gradient's
    /// `from="#000" to="#fff"`, adding it and its sections if they're missing.
    pub fn set_node(&mut self, path: &[&str], args: Vec<Value>, properties: Vec<(String, Value)>) {
        let nodes = self.nodes();
        let values = Values {
            args: &args,
            properties: &properties,
        };
        let head = head_text(path[path.len() - 1], values);

        if let Some(node) = find(&nodes, path, "").last() {
            self.text.replace_range(node.start..node.head_end, &head);
            return;
        }

        // Add it to the deepest section that exists, creating the ones in between.
        for depth in (1..path.len()).rev() {
            if let Some(parent) = find(&nodes, &path[..depth], "").last() {
                self.insert_child(parent, &path[depth..], values);
                return;
            }
        }

        // Nothing of the path exists: add the whole thing at the end.
        if !self.text.is_empty() {
            if !self.text.ends_with('\n') {
                self.text.push('\n');
            }
            self.text.push('\n');
        }
        self.text.push_str(&block(path, values, ""));
        self.text.push('\n');
    }

    /// Removes every node at `path` that bento would use. Returns whether there was one.
    pub fn remove(&mut self, path: &[&str]) -> bool {
        let nodes = self.nodes();
        let mut ranges: Vec<(usize, usize)> = find(&nodes, path, "")
            .iter()
            .map(|node| removal_range(&self.text, node))
            .collect();
        ranges.sort_unstable();
        for (start, end) in ranges.iter().rev() {
            self.text.replace_range(start..end, "");
        }
        !ranges.is_empty()
    }

    fn insert_child(&mut self, parent: &Node, rest: &[&str], values: Values) {
        let parent_indent = indent_at(&self.text, parent.start).to_owned();
        let Some((_, close)) = parent.braces else {
            // A node without a block, like `border`: give it one.
            let child_indent = format!("{parent_indent}    ");
            let text = format!(
                " {{\n{}\n{parent_indent}}}",
                block(rest, values, &child_indent)
            );
            self.text.insert_str(parent.head_end, &text);
            return;
        };

        let close_line = line_start(&self.text, close);
        if self.text[close_line..close].trim().is_empty() {
            // `}` on its own line: add a line above it, indented like the other children.
            let child_indent = match parent.children.last() {
                Some(child) => indent_at(&self.text, child.start).to_owned(),
                None => format!("{parent_indent}    "),
            };
            let text = format!("{}\n", block(rest, values, &child_indent));
            self.text.insert_str(close_line, &text);
        } else {
            // A block on one line, like `touchpad { tap; }`: keep it on one line.
            let space = if self.text[..close].ends_with(' ') {
                ""
            } else {
                " "
            };
            let text = format!("{space}{}; ", inline(rest, values));
            self.text.insert_str(close, &text);
        }
    }
}

/// A bind as a file writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct BindNode {
    /// The key as written, like `Mod+T`.
    pub key: String,
    /// Properties of the bind itself, like `repeat=false`.
    pub properties: Vec<(String, Value)>,
    /// Its action, or `None` for a bind with `press` and `release` sections.
    pub action: Option<ActionNode>,
}

/// An action as a bind writes it, like `spawn "foot"` or `move-window-to-workspace 2 focus=false`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionNode {
    pub name: String,
    pub args: Vec<Value>,
    pub properties: Vec<(String, Value)>,
}

impl ActionNode {
    pub fn text(&self) -> String {
        let mut out = String::new();
        write_ident(&mut out, &self.name);
        let args = self.arguments();
        if !args.is_empty() {
            out.push(' ');
            out.push_str(&args);
        }
        out
    }

    /// The arguments and properties, like `2 focus=false`.
    pub fn arguments(&self) -> String {
        let mut out = String::new();
        for value in &self.args {
            out.push(' ');
            write_value(&mut out, value);
        }
        for (name, value) in &self.properties {
            out.push(' ');
            write_ident(&mut out, name);
            out.push('=');
            write_value(&mut out, value);
        }
        out.trim_start().to_owned()
    }
}

impl BindNode {
    /// The bind as one line, like `Mod+T repeat=false { spawn "foot"; }`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        write_ident(&mut out, &self.key);
        for (name, value) in &self.properties {
            out.push(' ');
            write_ident(&mut out, name);
            out.push('=');
            write_value(&mut out, value);
        }
        let action = self
            .action
            .as_ref()
            .map_or_else(String::new, ActionNode::text);
        write!(out, " {{ {action}; }}").unwrap();
        out
    }
}

impl Document {
    /// The binds in the document's `binds` sections, in order.
    pub fn binds(&self) -> Vec<BindNode> {
        let doc = knuffel::parse_ast::<Span>("config.kdl", &self.text)
            .expect("edits keep the document valid");
        let sections = doc.nodes.iter().filter(|node| &**node.node_name == "binds");
        let binds = sections.flat_map(|section| section.children.iter().flat_map(|c| c.iter()));
        binds.map(bind_node).collect()
    }

    /// Replaces the bind for `key`, however the file spells it, or adds it to the last `binds`
    /// section.
    pub fn set_bind(&mut self, key: &Key, bind: &BindNode) {
        let text = bind.text();
        if let Some(node) = self.find_bind(key) {
            // From the key to its closing brace, keeping the terminator after it.
            let end = node.braces.map_or(node.head_end, |(_, close)| close + 1);
            self.text.replace_range(node.start..end, &text);
            return;
        }

        let nodes = self.nodes();
        match nodes.iter().rfind(|node| node.name == "binds") {
            Some(section) => self.insert_line(section, &text),
            None => {
                if !self.text.is_empty() && !self.text.ends_with('\n') {
                    self.text.push('\n');
                }
                if !self.text.is_empty() {
                    self.text.push('\n');
                }
                write!(self.text, "binds {{\n    {text}\n}}\n").unwrap();
            }
        }
    }

    /// Removes the bind for `key`, however the file spells it. Returns whether there was one.
    pub fn remove_bind(&mut self, key: &Key) -> bool {
        let Some(node) = self.find_bind(key) else {
            return false;
        };
        let (start, end) = removal_range(&self.text, &node);
        self.text.replace_range(start..end, "");
        true
    }

    /// Replaces every `binds` section with `section`, a whole `binds {}` section of its own.
    pub fn replace_binds(&mut self, section: &str) {
        let nodes = self.nodes();
        let mut ranges: Vec<(usize, usize)> = nodes
            .iter()
            .filter(|node| node.name == "binds")
            .map(|node| removal_range(&self.text, node))
            .collect();
        ranges.sort_unstable();
        for (start, end) in ranges.iter().rev() {
            self.text.replace_range(start..end, "");
        }
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
        self.text.push_str(section.trim_end());
        self.text.push('\n');
    }

    /// The text of the first `binds` section in the document.
    pub fn binds_section(&self) -> Option<String> {
        let nodes = self.nodes();
        let section = nodes.iter().find(|node| node.name == "binds")?;
        let end = section
            .braces
            .map_or(section.head_end, |(_, close)| close + 1);
        Some(self.text[section.start..end].to_owned())
    }

    fn find_bind(&self, key: &Key) -> Option<Node> {
        let nodes = self.nodes();
        let sections = nodes.iter().filter(|node| node.name == "binds");
        let mut binds = sections.flat_map(|section| section.children.iter());
        binds
            .rfind(|node| node.name.parse::<Key>().is_ok_and(|k| k == *key))
            .cloned()
    }

    /// Adds `text` as the last line of `section`.
    fn insert_line(&mut self, section: &Node, text: &str) {
        let indent = match section.children.last() {
            Some(child) => indent_at(&self.text, child.start).to_owned(),
            None => format!("{}    ", indent_at(&self.text, section.start)),
        };
        match section.braces {
            Some((_, close)) => {
                let close_line = line_start(&self.text, close);
                if self.text[close_line..close].trim().is_empty() {
                    self.text
                        .insert_str(close_line, &format!("{indent}{text}\n"));
                } else {
                    self.text.insert_str(close, &format!(" {text} "));
                }
            }
            None => self
                .text
                .insert_str(section.head_end, &format!(" {{\n{indent}{text}\n}}")),
        }
    }
}

fn bind_node(node: &ast::SpannedNode<Span>) -> BindNode {
    let children: Vec<_> = node.children.iter().flat_map(|c| c.iter()).collect();
    let action = match children.as_slice() {
        [action] if !matches!(&**action.node_name, "press" | "release") => Some(ActionNode {
            name: action.node_name.to_string(),
            args: action
                .arguments
                .iter()
                .filter_map(|v| value(&v.literal))
                .collect(),
            properties: properties(action),
        }),
        _ => None,
    };
    BindNode {
        key: node.node_name.to_string(),
        properties: properties(node),
        action,
    }
}

fn properties(node: &ast::SpannedNode<Span>) -> Vec<(String, Value)> {
    let props = node.properties.iter();
    props
        .filter_map(|(name, v)| Some((name.to_string(), value(&v.literal)?)))
        .collect()
}

fn value(literal: &ast::Literal) -> Option<Value> {
    Some(match literal {
        ast::Literal::Bool(x) => Value::Bool(*x),
        ast::Literal::Int(x) => Value::Int(i64::try_from(x).ok()?),
        ast::Literal::Decimal(x) => Value::Float(f64::try_from(x).ok()?),
        ast::Literal::String(x) => Value::Str(x.to_string()),
        ast::Literal::Null => return None,
    })
}

fn locate(node: &ast::SpannedNode<Span>, text: &str) -> Node {
    let start = node.node_name.span().0;
    let mut head_end = node.node_name.span().1;
    for value in &node.arguments {
        head_end = head_end.max(value.literal.span().1);
    }
    for value in node.properties.values() {
        head_end = head_end.max(value.literal.span().1);
    }
    let braces = node
        .children
        .as_ref()
        .map(|children| (children.span().0, children.span().1 - 1));
    debug_assert!(braces.is_none_or(|(_, close)| text.as_bytes()[close] == b'}'));
    Node {
        name: node.node_name.to_string(),
        start,
        head_end,
        end: node.span().1,
        braces,
        children: node
            .children
            .as_ref()
            .map(|children| children.iter().map(|child| locate(child, text)).collect())
            .unwrap_or_default(),
    }
}

/// The nodes at `path` that bento uses, in document order.
fn find<'a>(nodes: &'a [Node], path: &[&str], prefix: &str) -> Vec<&'a Node> {
    let Some((first, rest)) = path.split_first() else {
        return Vec::new();
    };
    let full = if prefix.is_empty() {
        first.to_string()
    } else {
        format!("{prefix}.{first}")
    };

    let mut candidates: Vec<&Node> = nodes
        .iter()
        .filter(|node| node.name == *first)
        .filter(|node| !RULES.contains(first) || applies_to_everything(node))
        .collect();
    if WHOLE_SECTIONS.contains(&full.as_str()) {
        candidates = candidates.split_off(candidates.len().saturating_sub(1));
    }

    if rest.is_empty() {
        return candidates;
    }

    // A section replaced as a whole further down, like `touchpad` under `input`: only its last
    // copy counts, wherever it is, so don't fall back to an earlier one that has the setting.
    let target = format!("{full}.{}", rest.join("."));
    let whole_below = WHOLE_SECTIONS.iter().find(|section| {
        section.starts_with(&format!("{full}."))
            && (target == **section || target.starts_with(&format!("{section}.")))
    });
    if let Some(section) = whole_below {
        let section_rest: Vec<&str> = section[full.len() + 1..].split('.').collect();
        return candidates
            .iter()
            .rev()
            .find(|node| !find(&node.children, &section_rest, &full).is_empty())
            .map(|node| find(&node.children, rest, &full))
            .unwrap_or_default();
    }

    // Where several sections have the setting, the last one wins.
    candidates
        .iter()
        .rev()
        .map(|node| find(&node.children, rest, &full))
        .find(|found| !found.is_empty())
        .unwrap_or_default()
}

fn applies_to_everything(rule: &Node) -> bool {
    !rule
        .children
        .iter()
        .any(|child| child.name == "match" || child.name == "exclude")
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn indent_at(text: &str, pos: usize) -> &str {
    let line = &text[line_start(text, pos)..pos];
    &line[..line.len() - line.trim_start().len()]
}

/// What to delete to remove a node: its whole line when it's alone on it.
fn removal_range(text: &str, node: &Node) -> (usize, usize) {
    let line = line_start(text, node.start);
    let alone_before = text[line..node.start].trim().is_empty();
    // The terminator ends at a newline, or at a `;` with more of the line after it.
    let ends_line = text[..node.end].ends_with('\n');
    let rest_of_line = if ends_line {
        ""
    } else {
        text[node.end..].split('\n').next().unwrap_or_default()
    };
    if alone_before && rest_of_line.trim().is_empty() {
        let mut end = node.end + rest_of_line.len();
        if !ends_line && text[end..].starts_with('\n') {
            end += 1;
        }
        (line, end)
    } else {
        // Shares its line: take the node and the spaces after it.
        let after = &text[node.end..];
        (
            node.start,
            node.end + after.len() - after.trim_start_matches(' ').len(),
        )
    }
}

fn head_text(name: &str, values: Values) -> String {
    let mut out = String::new();
    write_ident(&mut out, name);
    for arg in values.args {
        out.push(' ');
        write_value(&mut out, arg);
    }
    for (name, value) in values.properties {
        out.push(' ');
        write_ident(&mut out, name);
        out.push('=');
        write_value(&mut out, value);
    }
    out
}

/// A path as nested sections, one node per line.
fn block(path: &[&str], values: Values, indent: &str) -> String {
    match path {
        [] => String::new(),
        [leaf] => format!("{indent}{}", head_text(leaf, values)),
        [first, rest @ ..] => format!(
            "{indent}{} {{\n{}\n{indent}}}",
            head_text(first, Values::NONE),
            block(rest, values, &format!("{indent}    "))
        ),
    }
}

/// A path as nested sections on one line.
fn inline(path: &[&str], values: Values) -> String {
    match path {
        [] => String::new(),
        [leaf] => head_text(leaf, values),
        [first, rest @ ..] => format!(
            "{} {{ {}; }}",
            head_text(first, Values::NONE),
            inline(rest, values)
        ),
    }
}

fn write_ident(out: &mut String, name: &str) {
    let bare = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        && !matches!(name, "true" | "false" | "null");
    if bare {
        out.push_str(name);
    } else {
        write_string(out, name);
    }
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Bool(x) => write!(out, "{x}").unwrap(),
        Value::Int(x) => write!(out, "{x}").unwrap(),
        // KDL needs a decimal point to read a number as a decimal.
        Value::Float(x) if x.fract() == 0. && x.abs() < 1e15 => write!(out, "{x:.1}").unwrap(),
        Value::Float(x) => write!(out, "{x}").unwrap(),
        Value::Str(x) => write_string(out, x),
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &str, f: impl FnOnce(&mut Document)) -> String {
        let mut doc = Document::parse(text.to_owned()).unwrap();
        f(&mut doc);
        Document::parse(doc.text().to_owned()).expect("edit must stay valid KDL");
        doc.text().to_owned()
    }

    fn key(text: &str) -> Key {
        text.parse().unwrap()
    }

    fn spawn(key: &str, command: &str) -> BindNode {
        BindNode {
            key: key.to_owned(),
            properties: Vec::new(),
            action: Some(ActionNode {
                name: String::from("spawn"),
                args: vec![Value::Str(command.to_owned())],
                properties: Vec::new(),
            }),
        }
    }

    #[test]
    fn reads_binds_with_their_properties() {
        let text = "binds {\n    Mod+T repeat=false { spawn \"foot\"; }\n    Mod+2 { focus-workspace 2; }\n    Mod+Q { press { close-window; }; }\n}\n";
        let binds = Document::parse(text.to_owned()).unwrap().binds();
        assert_eq!(binds[0].key, "Mod+T");
        assert_eq!(
            binds[0].properties,
            [(String::from("repeat"), Value::Bool(false))]
        );
        assert_eq!(binds[0].action.as_ref().unwrap().text(), "spawn \"foot\"");
        assert_eq!(binds[1].action.as_ref().unwrap().args, [Value::Int(2)]);
        assert_eq!(binds[2].action, None);
    }

    #[test]
    fn sets_a_bind_in_place_however_the_key_is_spelled() {
        let text = "binds {\n    // Terminal.\n    Super+t { spawn \"foot\"; }\n    Mod+Q { close-window; }\n}\n";
        let out = edit(text, |doc| {
            doc.set_bind(&key("Super+T"), &spawn("Super+T", "kitty"))
        });
        assert_eq!(
            out,
            "binds {\n    // Terminal.\n    Super+T { spawn \"kitty\"; }\n    Mod+Q { close-window; }\n}\n"
        );
    }

    #[test]
    fn adds_a_bind_to_the_last_section_or_a_new_one() {
        let out = edit("binds {\n    Mod+Q { close-window; }\n}\n", |doc| {
            doc.set_bind(&key("Mod+T"), &spawn("Mod+T", "foot"))
        });
        assert_eq!(
            out,
            "binds {\n    Mod+Q { close-window; }\n    Mod+T { spawn \"foot\"; }\n}\n"
        );

        let out = edit("layout {\n    gaps 8\n}\n", |doc| {
            doc.set_bind(&key("Mod+T"), &spawn("Mod+T", "foot"))
        });
        assert_eq!(
            out,
            "layout {\n    gaps 8\n}\n\nbinds {\n    Mod+T { spawn \"foot\"; }\n}\n"
        );
    }

    #[test]
    fn removes_a_bind_and_its_line() {
        let text = "binds {\n    Mod+T { spawn \"foot\"; }\n    Mod+Q { close-window; }\n}\n";
        let out = edit(text, |doc| {
            assert!(doc.remove_bind(&key("Mod+T")));
        });
        assert_eq!(out, "binds {\n    Mod+Q { close-window; }\n}\n");
    }

    #[test]
    fn replaces_every_binds_section() {
        let text = "binds {\n    Mod+T { spawn \"foot\"; }\n}\nlayout { gaps 8; }\nbinds {\n    Mod+Q { close-window; }\n}\n";
        let out = edit(text, |doc| {
            doc.replace_binds("binds {\n    Mod+W { close-window; }\n}")
        });
        assert_eq!(
            out,
            "layout { gaps 8; }\nbinds {\n    Mod+W { close-window; }\n}\n"
        );
    }

    #[test]
    fn replaces_a_value_and_keeps_comments() {
        let text = "// Layout.\nlayout {\n    gaps 16 // between windows\n\n    // Focus.\n    focus-ring { width 4; }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("layout.gaps"), vec![Value::Int(8)])
        });
        assert_eq!(out, text.replace("gaps 16", "gaps 8"));
    }

    #[test]
    fn adds_a_line_at_the_section_indent() {
        let text = "input {\n  touchpad {\n    tap\n  }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("input.touchpad.dwt"), Vec::new())
        });
        assert_eq!(out, "input {\n  touchpad {\n    tap\n    dwt\n  }\n}\n");
    }

    #[test]
    fn adds_to_a_block_on_one_line() {
        let out = edit("layout { gaps 4; }\n", |doc| {
            doc.set(&split("layout.focus-ring.width"), vec![Value::Float(2.5)])
        });
        assert_eq!(out, "layout { gaps 4; focus-ring { width 2.5; }; }\n");
    }

    #[test]
    fn creates_missing_sections() {
        let out = edit(
            "input {\n    keyboard {\n        numlock\n    }\n}\n",
            |doc| {
                doc.set(
                    &split("input.touchpad.accel-speed"),
                    vec![Value::Float(0.5)],
                )
            },
        );
        assert_eq!(
            out,
            "input {\n    keyboard {\n        numlock\n    }\n    touchpad {\n        accel-speed 0.5\n    }\n}\n"
        );

        let out = edit("// Nothing yet.\n", |doc| {
            doc.set(&split("cursor.xcursor-size"), vec![Value::Int(32)])
        });
        assert_eq!(out, "// Nothing yet.\n\ncursor {\n    xcursor-size 32\n}\n");
    }

    #[test]
    fn writes_properties_after_the_arguments() {
        let gradient = vec![
            (String::from("from"), Value::Str(String::from("#000000"))),
            (String::from("to"), Value::Str(String::from("#ffffff"))),
            (String::from("angle"), Value::Int(45)),
            (
                String::from("relative-to"),
                Value::Str(String::from("view")),
            ),
            (String::from("in"), Value::Str(String::from("oklab"))),
        ];
        let path = split("options.insert-hint-gradient");

        // A node that's added, in a section that exists and in one that doesn't.
        let out = edit("options {\n    insert-hint true\n}\n", |doc| {
            doc.set_node(&path, Vec::new(), gradient.clone())
        });
        assert_eq!(
            out,
            "options {\n    insert-hint true\n    insert-hint-gradient from=\"#000000\" to=\"#ffffff\" angle=45 relative-to=\"view\" in=\"oklab\"\n}\n"
        );
        let out = edit("", |doc| doc.set_node(&path, Vec::new(), gradient.clone()));
        assert!(out.starts_with("options {\n    insert-hint-gradient from="));

        // One that's replaced loses the properties it had.
        let text =
            "options {\n    insert-hint-gradient from=\"#111\" to=\"#222\" angle=90 // mine\n}\n";
        let out = edit(text, |doc| {
            doc.set_node(
                &path,
                Vec::new(),
                vec![
                    (String::from("from"), Value::Str(String::from("#000000"))),
                    (String::from("to"), Value::Str(String::from("#ffffff"))),
                ],
            )
        });
        assert_eq!(
            out,
            "options {\n    insert-hint-gradient from=\"#000000\" to=\"#ffffff\" // mine\n}\n"
        );

        // Without properties it's a bare node again.
        let out = edit(text, |doc| doc.set(&path, Vec::new()));
        assert_eq!(out, "options {\n    insert-hint-gradient // mine\n}\n");
    }

    #[test]
    fn gives_a_bare_node_a_block() {
        let out = edit("layout {\n    border\n}\n", |doc| {
            doc.set(&split("layout.border.width"), vec![Value::Int(2)])
        });
        assert_eq!(out, "layout {\n    border {\n        width 2\n    }\n}\n");
    }

    #[test]
    fn removes_whole_lines() {
        let text = "input {\n    touchpad {\n        tap\n        natural-scroll // yes\n        dwt\n    }\n}\n";
        let out = edit(text, |doc| {
            assert!(doc.remove(&split("input.touchpad.natural-scroll")));
        });
        assert_eq!(
            out,
            "input {\n    touchpad {\n        tap\n        dwt\n    }\n}\n"
        );

        let out = edit("touchpad { tap; dwt; }\n", |doc| {
            assert!(doc.remove(&["touchpad", "tap"]));
        });
        assert_eq!(out, "touchpad { dwt; }\n");
        assert!(!Document::parse(out).unwrap().remove(&["touchpad", "tap"]));
    }

    #[test]
    fn edits_the_section_bento_uses() {
        // Merged sections: the last one with the setting.
        let text = "layout {\n    gaps 1\n}\nlayout {\n    gaps 2\n}\nlayout {\n    struts {}\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("layout.gaps"), vec![Value::Int(3)])
        });
        assert_eq!(out, text.replace("gaps 2", "gaps 3"));

        // Sections replaced as a whole: only the last one, even if an earlier one has it.
        let text = "input {\n    touchpad {\n        tap\n    }\n}\ninput {\n    touchpad {\n        dwt\n    }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("input.touchpad.tap"), Vec::new())
        });
        assert_eq!(
            out,
            "input {\n    touchpad {\n        tap\n    }\n}\ninput {\n    touchpad {\n        dwt\n        tap\n    }\n}\n"
        );
    }

    #[test]
    fn edits_only_rules_for_every_window() {
        let text = "window-rule {\n    geometry-corner-radius 12\n}\nwindow-rule {\n    match app-id=\"foot\"\n    geometry-corner-radius 0\n}\n";
        let out = edit(text, |doc| {
            doc.set(
                &split("window-rule.geometry-corner-radius"),
                vec![Value::Int(15)],
            )
        });
        assert_eq!(out, text.replacen("radius 12", "radius 15", 1));
    }

    #[test]
    fn positions_are_bytes() {
        let text = "// fæ ✨\nlayout {\n    gaps 16\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("layout.gaps"), vec![Value::Int(8)])
        });
        assert_eq!(out, "// fæ ✨\nlayout {\n    gaps 8\n}\n");
    }

    #[test]
    fn writes_values() {
        let out = edit("", |doc| {
            doc.set(&["a"], vec![Value::Float(2.)]);
            doc.set(&["b"], vec![Value::Str(String::from("x \"y\" \\z"))]);
            doc.set(&["Mod+T"], vec![Value::Bool(false)]);
        });
        assert_eq!(out, "a 2.0\n\nb \"x \\\"y\\\" \\\\z\"\n\nMod+T false\n");
    }
}
