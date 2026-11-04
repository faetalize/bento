//! A small KDL document model for the files this app reads and writes.
//!
//! knuffel parses the text; the app then edits plain `Node` trees and writes them back out. Type
//! annotations and the order of properties aren't kept, since bento's config doesn't use them.

use std::fmt::Write as _;

use knuffel::ast::{self, Literal};
use knuffel::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(x) => Some(*x as f64),
            Value::Float(x) => Some(*x),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(x) => Some(x),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub args: Vec<Value>,
    pub props: Vec<(String, Value)>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    pub fn arg(&self) -> Option<&Value> {
        self.args.first()
    }

    fn is_empty(&self) -> bool {
        self.args.is_empty() && self.props.is_empty() && self.children.is_empty()
    }
}

pub fn parse(file_name: &str, text: &str) -> Result<Vec<Node>, knuffel::Error> {
    let doc = knuffel::parse_ast::<Span>(file_name, text)?;
    Ok(doc.nodes.iter().map(|node| from_ast(node)).collect())
}

fn from_ast(node: &ast::Node<Span>) -> Node {
    Node {
        name: node.node_name.to_string(),
        args: node
            .arguments
            .iter()
            .map(|value| from_literal(&value.literal))
            .collect(),
        props: node
            .properties
            .iter()
            .map(|(name, value)| (name.to_string(), from_literal(&value.literal)))
            .collect(),
        children: node
            .children
            .as_ref()
            .map(|children| children.iter().map(|child| from_ast(child)).collect())
            .unwrap_or_default(),
    }
}

fn from_literal(literal: &Literal) -> Value {
    match literal {
        Literal::Null => Value::Null,
        Literal::Bool(x) => Value::Bool(*x),
        Literal::Int(x) => i64::try_from(x).map_or(Value::Null, Value::Int),
        Literal::Decimal(x) => f64::try_from(x).map_or(Value::Null, Value::Float),
        Literal::String(x) => Value::Str(x.to_string()),
    }
}

pub fn to_string(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        write_node(&mut out, node, 0);
    }
    out
}

fn write_node(out: &mut String, node: &Node, depth: usize) {
    out.push_str(&"    ".repeat(depth));
    write_ident(out, &node.name);
    for arg in &node.args {
        out.push(' ');
        write_value(out, arg);
    }
    for (name, value) in &node.props {
        out.push(' ');
        write_ident(out, name);
        out.push('=');
        write_value(out, value);
    }
    if !node.children.is_empty() {
        out.push_str(" {\n");
        for child in &node.children {
            write_node(out, child, depth + 1);
        }
        out.push_str(&"    ".repeat(depth));
        out.push('}');
    }
    out.push('\n');
}

fn write_ident(out: &mut String, name: &str) {
    let bare = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !matches!(name, "true" | "false" | "null");
    if bare {
        out.push_str(name);
    } else {
        write_string(out, name);
    }
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
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

/// Splits a dotted path like `layout.focus-ring.width`.
pub fn split(path: &str) -> Vec<&str> {
    path.split('.').collect()
}

/// Finds the node at a path. Where a name repeats, the last one wins, as in bento's config.
pub fn get<'a>(nodes: &'a [Node], path: &[&str]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    let node = nodes.iter().rev().find(|node| node.name == *first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        get(&node.children, rest)
    }
}

/// Finds the node at a path, creating it and its parents if they don't exist.
pub fn ensure<'a>(nodes: &'a mut Vec<Node>, path: &[&str]) -> &'a mut Node {
    let (first, rest) = path.split_first().expect("path must not be empty");
    let idx = match nodes.iter().rposition(|node| node.name == *first) {
        Some(idx) => idx,
        None => {
            nodes.push(Node::new(first));
            nodes.len() - 1
        }
    };
    let node = &mut nodes[idx];
    if rest.is_empty() {
        node
    } else {
        ensure(&mut node.children, rest)
    }
}

/// Removes every node at a path, then any parents left empty by it. Returns whether it removed
/// anything.
pub fn remove(nodes: &mut Vec<Node>, path: &[&str]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    if rest.is_empty() {
        let before = nodes.len();
        nodes.retain(|node| node.name != *first);
        return nodes.len() != before;
    }

    let mut removed = false;
    for node in nodes.iter_mut().filter(|node| node.name == *first) {
        removed |= remove(&mut node.children, rest);
    }
    if removed {
        nodes.retain(|node| node.name != *first || !node.is_empty());
    }
    removed
}

/// Sets the arguments of the node at a path, replacing any it had.
pub fn set_args(nodes: &mut Vec<Node>, path: &[&str], args: Vec<Value>) {
    let node = ensure(nodes, path);
    node.args = args;
    node.props.clear();
    node.children.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "layout {\n    gaps 16\n    focus-ring {\n        off\n        active-color \"#7fc8ff\"\n    }\n}\ninclude \"a b.kdl\" optional=true\n";
        let nodes = parse("test.kdl", text).unwrap();
        assert_eq!(to_string(&nodes), text);
    }

    #[test]
    fn writes_values() {
        let mut nodes = Vec::new();
        set_args(&mut nodes, &["a"], vec![Value::Float(2.)]);
        set_args(&mut nodes, &["b"], vec![Value::Float(0.25), Value::Int(-3)]);
        set_args(
            &mut nodes,
            &["c"],
            vec![Value::Str(String::from("x \"y\" \\z"))],
        );
        set_args(&mut nodes, &["d e"], vec![Value::Bool(false), Value::Null]);
        assert_eq!(
            to_string(&nodes),
            "a 2.0\nb 0.25 -3\nc \"x \\\"y\\\" \\\\z\"\n\"d e\" false null\n"
        );
        assert_eq!(parse("test.kdl", &to_string(&nodes)).unwrap(), nodes);
    }

    #[test]
    fn edits_paths() {
        let mut nodes = parse(
            "test.kdl",
            "input {\n    keyboard {\n        numlock\n    }\n}\n",
        )
        .unwrap();
        set_args(
            &mut nodes,
            &split("input.touchpad.accel-speed"),
            vec![Value::Float(0.5)],
        );
        assert_eq!(
            get(&nodes, &split("input.touchpad.accel-speed"))
                .unwrap()
                .arg(),
            Some(&Value::Float(0.5))
        );

        assert!(remove(&mut nodes, &split("input.touchpad.accel-speed")));
        assert!(get(&nodes, &["input", "touchpad"]).is_none());
        assert!(get(&nodes, &split("input.keyboard.numlock")).is_some());

        assert!(remove(&mut nodes, &split("input.keyboard.numlock")));
        assert!(nodes.is_empty());
        assert!(!remove(&mut nodes, &["input"]));
    }

    #[test]
    fn last_node_wins() {
        let nodes = parse("test.kdl", "layout { gaps 1; }\nlayout { gaps 2; }\n").unwrap();
        assert_eq!(
            get(&nodes, &["layout", "gaps"]).unwrap().arg(),
            Some(&Value::Int(2))
        );
    }
}
