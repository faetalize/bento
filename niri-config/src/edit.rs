//! Edits to a hand-written KDL config that leave everything else in it alone.
//!
//! knuffel parses the text with byte positions for every node and value. A change replaces only
//! the node it's about, or adds one line to the section it belongs in at the section's own
//! indentation, so comments, ordering and spacing stay the way the person wrote them.
//!
//! Paths like `input.touchpad.tap` find the node bento actually uses: where a section appears more
//! than once, the last one that has the setting wins, since later sections override earlier ones.
//! Sections that bento replaces as a whole only count in their last copy, and window rules only
//! count when they apply to every window. Nodes that come as a list, like glass's lights, are
//! reached by their place in it, as in `blur.glass.light.1.size`.

use std::fmt::Write as _;

use knuffel::ast;
use knuffel::span::Span;

use crate::binds::UNBIND;
use crate::Key;

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

/// Nodes that come as a list, where the section with any of them gives all of them, and later
/// sections without any leave them alone. A path reaches one by its place, like
/// `blur.glass.light.1`, and a new one goes after the others.
const LISTS: &[&str] = &["blur.glass.light"];

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

/// A node to write as a child, with its own children.
#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    pub name: String,
    pub args: Vec<Value>,
    pub children: Vec<Child>,
}

impl Child {
    /// The child as lines at `indent`, its children a step further in.
    fn text(&self, indent: &str) -> String {
        let values = Values {
            args: &self.args,
            properties: &[],
        };
        let mut out = format!("{indent}{}", head_text(&self.name, values));
        if !self.children.is_empty() {
            out.push_str(&children_text(&self.children, indent));
        }
        out
    }
}

/// `children` as a block after a node at `indent`: ` {`, a line for each, and `}`.
fn children_text(children: &[Child], indent: &str) -> String {
    let inner = format!("{indent}    ");
    let mut out = String::from(" {\n");
    for child in children {
        out.push_str(&child.text(&inner));
        out.push('\n');
    }
    out.push_str(indent);
    out.push('}');
    out
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
        let index = list_index(path);
        // A list item named by its place is named like the list.
        let name = match index {
            Some(i) if i == path.len() - 1 => path[i - 1],
            _ => path[path.len() - 1],
        };
        let head = head_text(name, values);

        if let Some(node) = find(&nodes, path, "").last() {
            self.text.replace_range(node.start..node.head_end, &head);
            return;
        }

        // Add it to the deepest section that exists, creating the ones in between.
        for depth in (1..path.len()).rev() {
            if index == Some(depth) {
                // A list item that isn't there: add it after the others, or start the list.
                let mut item = vec![path[depth - 1]];
                item.extend_from_slice(&path[depth + 1..]);
                if let Some(owner) = list_owner(&nodes, &path[..depth - 1], path[depth - 1]) {
                    self.insert_child(owner, &item, values);
                } else {
                    let mut path = path[..depth].to_vec();
                    path.extend_from_slice(&item[1..]);
                    self.set_node(&path, args.clone(), properties.clone());
                }
                return;
            }
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

    /// Sets the arguments of the node at `path`, adding it if it's missing, and puts `children`
    /// in place of its children: none leaves it without a block.
    pub fn set_with_children(&mut self, path: &[&str], args: Vec<Value>, children: &[Child]) {
        self.set_node(path, args, Vec::new());
        let nodes = self.nodes();
        let node = find(&nodes, path, "")
            .last()
            .map(|node| (*node).clone())
            .expect("the node was just set");
        let end = match node.braces {
            Some((_, close)) => close + 1,
            None => node.head_end,
        };
        let text = if children.is_empty() {
            String::new()
        } else {
            children_text(children, indent_at(&self.text, node.start))
        };
        self.text.replace_range(node.head_end..end, &text);
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
        if modifiers_alone(&self.key) {
            write!(out, " {{ release {{ {action}; }}; }}").unwrap();
        } else {
            write!(out, " {{ {action}; }}").unwrap();
        }
        out
    }
}

/// Whether `key` is modifiers alone, like `Mod`: its bind runs as they're let go with nothing
/// pressed in between, since run as they're pressed it would run whenever they start another
/// shortcut. It's written and read as a `release` section.
fn modifiers_alone(key: &str) -> bool {
    key.parse::<Key>()
        .is_ok_and(|key| key.trigger.is_modifier())
}

impl Document {
    /// The binds in the document's `binds` sections, in order.
    pub fn binds(&self) -> Vec<BindNode> {
        let doc = knuffel::parse_ast::<Span>("config.kdl", &self.text)
            .expect("edits keep the document valid");
        let sections = doc.nodes.iter().filter(|node| &**node.node_name == "binds");
        let binds = sections.flat_map(|section| section.children.iter().flat_map(|c| c.iter()));
        binds
            .filter(|node| &**node.node_name != UNBIND)
            .map(bind_node)
            .collect()
    }

    /// The shortcuts the document's `binds` sections turn off with `unbind`, as written.
    pub fn unbinds(&self) -> Vec<ActionNode> {
        let doc = knuffel::parse_ast::<Span>("config.kdl", &self.text)
            .expect("edits keep the document valid");
        let sections = doc.nodes.iter().filter(|node| &**node.node_name == "binds");
        let nodes = sections.flat_map(|section| section.children.iter().flat_map(|c| c.iter()));
        nodes
            .filter(|node| &**node.node_name == UNBIND)
            .filter_map(|node| match children(node).as_slice() {
                [action] => Some(action_node(action)),
                _ => None,
            })
            .collect()
    }

    /// Turns the shortcut `action` off, with an `unbind` in the last `binds` section, or a new one.
    pub fn add_unbind(&mut self, action: &ActionNode) {
        if self.unbinds().contains(action) {
            return;
        }
        let text = format!("{UNBIND} {{ {}; }}", action.text());
        self.add_bind_line(&text);
    }

    /// Takes away the `unbind` of `action`, as written. Returns whether there was one.
    pub fn remove_unbind(&mut self, action: &ActionNode) -> bool {
        let doc = knuffel::parse_ast::<Span>("config.kdl", &self.text)
            .expect("edits keep the document valid");
        let nodes = self.nodes();
        let sections = doc.nodes.iter().zip(&nodes);
        for (section, located) in sections.filter(|(node, _)| &**node.node_name == "binds") {
            let nodes = section.children.iter().flat_map(|c| c.iter());
            for (child, at) in nodes.zip(&located.children) {
                let found = &**child.node_name == UNBIND
                    && matches!(children(child).as_slice(), [one] if action_node(one) == *action);
                if found {
                    let (start, end) = removal_range(&self.text, at);
                    self.text.replace_range(start..end, "");
                    return true;
                }
            }
        }
        false
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

        self.add_bind_line(&text);
    }

    /// Adds `text` as the last line of the last `binds` section, or of a new one.
    fn add_bind_line(&mut self, text: &str) {
        let nodes = self.nodes();
        match nodes.iter().rfind(|node| node.name == "binds") {
            Some(section) => self.insert_line(section, text),
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

    /// The names of the document's top-level blocks, like a shell file's modules, in order, each
    /// once.
    pub fn block_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for node in self.nodes() {
            if node.braces.is_some() && !names.contains(&node.name) {
                names.push(node.name);
            }
        }
        names
    }

    /// The contents of every top-level block called `name`, each as a document of its own, like
    /// a module's settings and binds in a shell file.
    pub fn blocks(&self, name: &str) -> Vec<Document> {
        self.nodes()
            .iter()
            .filter(|node| node.name == name)
            .filter_map(|node| {
                let (open, close) = node.braces?;
                Document::parse(dedent(&self.text[open + 1..close])).ok()
            })
            .collect()
    }

    /// Edits the contents of the last top-level block called `name` as a document of its own,
    /// made at the end if there's none. A block left empty is taken away.
    pub fn edit_block(&mut self, name: &str, f: impl FnOnce(&mut Document)) {
        let nodes = self.nodes();
        let found = nodes
            .iter()
            .rfind(|node| node.name == name && node.braces.is_some());
        let inner = found
            .and_then(|node| node.braces)
            .map(|(open, close)| dedent(&self.text[open + 1..close]))
            .unwrap_or_default();
        let Ok(mut doc) = Document::parse(inner) else {
            return;
        };
        f(&mut doc);
        // A `binds {}` left with nothing in it goes too.
        let mut empty: Vec<(usize, usize)> = doc
            .nodes()
            .iter()
            .filter(|node| node.name == "binds" && node.children.is_empty())
            .map(|node| removal_range(&doc.text, node))
            .collect();
        empty.sort_unstable();
        for (start, end) in empty.into_iter().rev() {
            doc.text.replace_range(start..end, "");
        }
        let contents = doc.text.trim();

        match found {
            Some(node) if contents.is_empty() => {
                let (start, end) = removal_range(&self.text, node);
                self.text.replace_range(start..end, "");
            }
            Some(node) => {
                let (open, close) = node.braces.expect("found with braces");
                let indent = format!("{}    ", indent_at(&self.text, node.start));
                let body = indented(contents, &indent);
                let outer = indent_at(&self.text, node.start).to_owned();
                self.text
                    .replace_range(open + 1..close, &format!("\n{body}\n{outer}"));
            }
            None if contents.is_empty() => (),
            None => {
                if !self.text.is_empty() && !self.text.ends_with('\n') {
                    self.text.push('\n');
                }
                if !self.text.is_empty() {
                    self.text.push('\n');
                }
                let body = indented(contents, "    ");
                write!(self.text, "{name} {{\n{body}\n}}\n").unwrap();
            }
        }
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
                    // A block on one line: the line ends with `;`, as `insert_child` writes it.
                    let space = if self.text[..close].ends_with(' ') {
                        ""
                    } else {
                        " "
                    };
                    self.text.insert_str(close, &format!("{space}{text}; "));
                }
            }
            None => self
                .text
                .insert_str(section.head_end, &format!(" {{\n{indent}{text}\n}}")),
        }
    }
}

fn children(node: &ast::SpannedNode<Span>) -> Vec<&ast::SpannedNode<Span>> {
    node.children.iter().flat_map(|c| c.iter()).collect()
}

/// An action node as written.
fn action_node(action: &ast::SpannedNode<Span>) -> ActionNode {
    ActionNode {
        name: action.node_name.to_string(),
        args: action
            .arguments
            .iter()
            .filter_map(|v| value(&v.literal))
            .collect(),
        properties: properties(action),
    }
}

fn bind_node(node: &ast::SpannedNode<Span>) -> BindNode {
    let key = node.node_name.to_string();
    let action = match children(node).as_slice() {
        [action] if !matches!(&**action.node_name, "press" | "release") => {
            Some(action_node(action))
        }
        // Modifiers alone run their action as they're let go.
        [release] if &**release.node_name == "release" && modifiers_alone(&key) => {
            match children(release).as_slice() {
                [action] => Some(action_node(action)),
                _ => None,
            }
        }
        _ => None,
    };
    BindNode {
        key,
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

    // An item of a list, by its place.
    if LISTS.contains(&full.as_str()) {
        if let Ok(index) = rest[0].parse::<usize>() {
            return match (candidates.get(index), &rest[1..]) {
                (None, _) => Vec::new(),
                (Some(item), []) => vec![item],
                (Some(item), rest) => find(&item.children, rest, &full),
            };
        }
    }

    // A section replaced as a whole further down, like `touchpad` under `input`: only its last
    // copy counts, wherever it is, so don't fall back to an earlier one that has the setting.
    let target = format!("{full}.{}", rest.join("."));
    let whole_below = WHOLE_SECTIONS.iter().chain(LISTS).find(|section| {
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

/// Where `path` gives a list item's place, like the `1` in `blur.glass.light.1.size`.
fn list_index(path: &[&str]) -> Option<usize> {
    (1..path.len()).find(|&i| {
        LISTS.contains(&path[..i].join(".").as_str()) && path[i].parse::<usize>().is_ok()
    })
}

/// The section at `path` that has the `list` bento uses: the last one with any of it.
fn list_owner<'a>(nodes: &'a [Node], path: &[&str], list: &str) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    let mut candidates = nodes.iter().rev().filter(|node| node.name == *first);
    if rest.is_empty() {
        candidates.find(|node| node.children.iter().any(|child| child.name == list))
    } else {
        candidates.find_map(|node| list_owner(&node.children, rest, list))
    }
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
/// `text` with the indentation its lines share taken off.
fn dedent(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let common = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for line in lines {
        out.push_str(line.get(common..).unwrap_or(line.trim_start()));
        out.push('\n');
    }
    out.trim().to_owned() + "\n"
}

/// `text` with `indent` before each of its lines that has anything on it.
fn indented(text: &str, indent: &str) -> String {
    let lines = text.lines().map(|line| {
        if line.trim().is_empty() {
            String::new()
        } else {
            format!("{indent}{line}")
        }
    });
    lines.collect::<Vec<_>>().join("\n")
}

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

/// A plain value for a layout engine option or action argument, or `None` for a list or a
/// gradient, which is written with [`option_properties`].
pub fn option_value(value: &niri_ipc::LayoutEngineOptionValue) -> Option<Value> {
    use niri_ipc::LayoutEngineOptionValue as OptionValue;

    Some(match value {
        OptionValue::Boolean(x) => Value::Bool(*x),
        OptionValue::Number(x) if x.fract() == 0. => Value::Int(*x as i64),
        OptionValue::Number(x) => Value::Float(*x),
        OptionValue::String(x) => Value::Str(x.clone()),
        OptionValue::Gradient(_) | OptionValue::List(_) | OptionValue::Group(_) => return None,
    })
}

/// The children a `Pins` option is written with: `app "firefox"` for an app, and
/// `group "Games" { app "steam"; }` for a group.
pub fn pin_children(value: &Option<niri_ipc::LayoutEngineOptionValue>) -> Vec<Child> {
    use niri_ipc::LayoutEngineOptionValue as OptionValue;

    let app = |id: &String| Child {
        name: String::from("app"),
        args: vec![Value::Str(id.clone())],
        children: Vec::new(),
    };
    let items = match value {
        Some(OptionValue::List(items)) => items.as_slice(),
        Some(item) => std::slice::from_ref(item),
        None => &[],
    };
    items
        .iter()
        .filter_map(|item| match item {
            OptionValue::String(id) => Some(app(id)),
            OptionValue::Group(group) => Some(Child {
                name: String::from("group"),
                args: vec![Value::Str(group.group.clone())],
                children: group.apps.iter().map(app).collect(),
            }),
            _ => None,
        })
        .collect()
}

/// The values an option is written with: one for a plain option, one each for a list, none for a
/// size left to the window or a gradient.
pub fn option_values(value: &Option<niri_ipc::LayoutEngineOptionValue>) -> Vec<Value> {
    use niri_ipc::LayoutEngineOptionValue as OptionValue;

    match value {
        None => Vec::new(),
        Some(OptionValue::List(items)) => items.iter().filter_map(option_value).collect(),
        Some(value) => option_value(value).into_iter().collect(),
    }
}

/// The properties an option is written with, which only a gradient has: `from`, `to`, and the
/// other properties it sets.
pub fn option_properties(
    value: &Option<niri_ipc::LayoutEngineOptionValue>,
) -> Vec<(String, Value)> {
    use niri_ipc::LayoutEngineOptionValue as OptionValue;

    let Some(OptionValue::Gradient(gradient)) = value else {
        return Vec::new();
    };
    let text = |text: &String| Value::Str(text.clone());
    let mut properties = vec![
        (String::from("from"), text(&gradient.from)),
        (String::from("to"), text(&gradient.to)),
    ];
    if let Some(angle) = gradient.angle {
        let angle = if angle.fract() == 0. {
            Value::Int(angle as i64)
        } else {
            Value::Float(angle)
        };
        properties.push((String::from("angle"), angle));
    }
    if let Some(relative_to) = &gradient.relative_to {
        properties.push((String::from("relative-to"), text(relative_to)));
    }
    if let Some(space) = &gradient.color_space {
        properties.push((String::from("in"), text(space)));
    }
    properties
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

    #[test]
    fn a_block_is_edited_as_a_document_of_its_own() {
        let bind = |key: &str, name: &str| BindNode {
            key: String::from(key),
            properties: Vec::new(),
            action: Some(ActionNode {
                name: String::from(name),
                args: Vec::new(),
                properties: Vec::new(),
            }),
        };
        // Into a block that's there, after its settings.
        let text = "sound {\n    pinned \"hdmi\"\n}\n";
        let out = edit(text, |doc| {
            doc.edit_block("sound", |block| {
                block.set_bind(&key("Mod+F1"), &bind("Mod+F1", "switch-audio-output"));
            })
        });
        assert_eq!(
            out,
            "sound {\n    pinned \"hdmi\"\n\n    binds {\n        Mod+F1 { switch-audio-output; }\n    }\n}\n"
        );
        let doc = Document::parse(out.clone()).unwrap();
        assert_eq!(doc.blocks("sound")[0].binds()[0].key, "Mod+F1");
        assert_eq!(doc.block_names(), ["sound"]);

        // A block that isn't there is made, and one left empty goes.
        let out = edit("", |doc| {
            doc.edit_block("clock", |block| {
                block.add_unbind(&bind("x", "show-clock").action.unwrap())
            })
        });
        assert_eq!(
            out,
            "clock {\n    binds {\n        unbind { show-clock; }\n    }\n}\n"
        );
        let out = edit(&out, |doc| {
            doc.edit_block("clock", |block| {
                let off = block.unbinds();
                block.remove_unbind(&off[0]);
            })
        });
        assert_eq!(out.trim(), "");
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
    fn unbinds_are_read_added_and_taken_away_apart_from_binds() {
        let text =
            "binds {\n    Mod+T { spawn \"foot\"; }\n    unbind { power-off-monitors; }\n}\n";
        let mut doc = Document::parse(text.to_owned()).unwrap();
        // They aren't binds.
        assert_eq!(doc.binds().len(), 1);
        let off = |name: &str, args: Vec<Value>| ActionNode {
            name: name.to_owned(),
            args,
            properties: Vec::new(),
        };
        assert_eq!(doc.unbinds(), [off("power-off-monitors", Vec::new())]);
        let start = off(
            "shell-action",
            vec![Value::Str(String::from("toggle-start-menu"))],
        );
        doc.add_unbind(&start);
        assert_eq!(
            doc.text(),
            "binds {\n    Mod+T { spawn \"foot\"; }\n    unbind { power-off-monitors; }\n    unbind { shell-action \"toggle-start-menu\"; }\n}\n"
        );
        assert!(doc.remove_unbind(&off("power-off-monitors", Vec::new())));
        assert!(!doc.remove_unbind(&off("power-off-monitors", Vec::new())));
        assert_eq!(doc.unbinds(), [start]);
        Document::parse(doc.text().to_owned()).expect("still valid");
    }

    #[test]
    fn a_bind_of_modifiers_alone_runs_as_they_are_let_go() {
        // Run as it's pressed, it would run whenever the modifier starts another shortcut.
        let out = edit("binds {\n}\n", |doc| {
            doc.set_bind(&key("Mod"), &spawn("Mod", "menu"))
        });
        assert_eq!(
            out,
            "binds {\n    Mod { release { spawn \"menu\"; }; }\n}\n"
        );
        // And it reads back as its action, to change.
        let binds = Document::parse(out).unwrap().binds();
        assert_eq!(binds[0].action.as_ref().unwrap().text(), "spawn \"menu\"");
        // A key with more than modifiers keeps its sections as they are.
        let text = "binds {\n    Mod+Q { release { close-window; }; }\n}\n";
        assert_eq!(
            Document::parse(text.to_owned()).unwrap().binds()[0].action,
            None
        );
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
    fn adds_a_bind_to_binds_on_one_line() {
        let out = edit("binds { Mod+Q { close-window; }; }\n", |doc| {
            doc.set_bind(&key("Mod+T"), &spawn("Mod+T", "foot"))
        });
        assert!(Document::parse(out.clone()).is_ok(), "{out}");
        assert!(out.contains("Mod+Q { close-window; }") && out.contains("spawn \"foot\""));
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

    #[test]
    fn edits_a_list_item_by_its_place() {
        let text = "blur {\n    glass {\n        light {\n            angle 90\n        }\n        // Fill.\n        light { off; angle 200; }\n    }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("blur.glass.light.1.angle"), vec![Value::Int(210)])
        });
        assert_eq!(out, text.replace("angle 200", "angle 210"));

        // A field the item doesn't have yet.
        let out = edit(text, |doc| {
            doc.set(&split("blur.glass.light.0.size"), vec![Value::Float(0.5)])
        });
        assert_eq!(
            out,
            text.replace("angle 90\n", "angle 90\n            size 0.5\n")
        );

        let out = edit(text, |doc| {
            assert!(doc.remove(&split("blur.glass.light.0")));
            assert!(!doc.remove(&split("blur.glass.light.1")));
        });
        assert_eq!(
            out,
            "blur {\n    glass {\n        // Fill.\n        light { off; angle 200; }\n    }\n}\n"
        );
    }

    #[test]
    fn adds_list_items_after_the_others() {
        let text = "blur {\n    glass {\n        light { angle 90; }\n    }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("blur.glass.light.1.angle"), vec![Value::Int(270)])
        });
        assert_eq!(
            out,
            "blur {\n    glass {\n        light { angle 90; }\n        light {\n            angle 270\n        }\n    }\n}\n"
        );

        // The first one starts the list, in a section that exists and in one that doesn't.
        let out = edit("blur {\n    glass {\n        bezel 8\n    }\n}\n", |doc| {
            doc.set(&split("blur.glass.light.0.off"), Vec::new())
        });
        assert_eq!(
            out,
            "blur {\n    glass {\n        bezel 8\n        light {\n            off\n        }\n    }\n}\n"
        );
        let out = edit("", |doc| doc.set(&split("blur.glass.light.0"), Vec::new()));
        assert_eq!(out, "blur {\n    glass {\n        light\n    }\n}\n");
    }

    #[test]
    fn edits_the_list_bento_uses() {
        // The last section with any of the list has it; a later one without leaves it be.
        let text = "blur {\n    glass {\n        light { angle 90; }\n    }\n}\nblur {\n    glass {\n        bezel 8\n    }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("blur.glass.light.0.angle"), vec![Value::Int(80)]);
            doc.set(&split("blur.glass.light.1.angle"), vec![Value::Int(270)]);
        });
        assert_eq!(
            out,
            "blur {\n    glass {\n        light { angle 80; }\n        light {\n            angle 270\n        }\n    }\n}\nblur {\n    glass {\n        bezel 8\n    }\n}\n"
        );

        // A later section with any replaces the whole list.
        let text = "blur {\n    glass {\n        light { angle 90; }\n        light { angle 180; }\n    }\n    glass {\n        light { angle 270; }\n    }\n}\n";
        let out = edit(text, |doc| {
            doc.set(&split("blur.glass.light.0.angle"), vec![Value::Int(0)])
        });
        assert_eq!(out, text.replace("angle 270", "angle 0"));
        let doc = Document::parse(text.to_owned()).unwrap();
        assert!(doc.get(&split("blur.glass.light.1")).is_none());
        assert_eq!(
            doc.get(&split("blur.glass.light.0")).map(|node| node.name),
            Some(String::from("light"))
        );
    }
}
