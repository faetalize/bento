//! The tree of nodes that a layout draws on a workspace, as the layout last described it.
//!
//! The prelude sends the changes as ops (see `resources/layouts/prelude/nodes.js`), and this keeps
//! the result: ids, kinds, properties and children. Nodes are boxes, text and images that the
//! layout creates, and windows, which bento creates with the window and which stay for as long as
//! the window is on the workspace. The renderer draws what's in the tree.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::protocol::TreeOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Root,
    Box,
    Text,
    Image,
    Window,
}

impl NodeKind {
    fn parse(kind: &str) -> Option<Self> {
        match kind {
            "box" => Some(Self::Box),
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub kind: NodeKind,
    /// The node's properties, as the layout set them: an object.
    pub props: Value,
    /// Children from the bottom to the top.
    pub children: Vec<String>,
    parent: Option<String>,
}

impl TreeNode {
    fn new(kind: NodeKind, props: Value) -> Self {
        let props = if props.is_object() {
            props
        } else {
            Value::Object(Map::new())
        };
        Self {
            kind,
            props,
            children: Vec::new(),
            parent: None,
        }
    }
}

/// A node with everything under it, to move to another workspace's tree.
#[derive(Debug)]
pub struct Subtree(Vec<(String, TreeNode)>);

impl Subtree {
    /// Ids of the nodes in it.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(id, _)| id.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Tree {
    root: String,
    nodes: HashMap<String, TreeNode>,
}

/// Id of a window's node.
pub fn window_node_id(window: u64) -> String {
    format!("w{window}")
}

impl Tree {
    pub fn new(workspace: u64) -> Self {
        let root = format!("r{workspace}");
        let mut nodes = HashMap::new();
        nodes.insert(root.clone(), TreeNode::new(NodeKind::Root, Value::Null));
        Self { root, nodes }
    }

    /// Id of the root node, which has the windows and everything the layout draws as its
    /// descendants.
    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn node(&self, id: &str) -> Option<&TreeNode> {
        self.nodes.get(id)
    }

    /// Ids of the root's descendants from the bottom to the top, with their nodes: the order to
    /// draw them in.
    pub fn draw_order(&self) -> Vec<(&str, &TreeNode)> {
        let mut order = Vec::new();
        self.visit(&self.root, &mut |id, node| order.push((id, node)));
        order
    }

    fn visit<'a>(&'a self, id: &'a str, f: &mut dyn FnMut(&'a str, &'a TreeNode)) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        f(id, node);
        for child in &node.children {
            self.visit(child, f);
        }
    }

    /// Ids of the windows in the tree from the bottom to the top.
    pub fn window_order(&self) -> Vec<u64> {
        self.draw_order()
            .into_iter()
            .filter(|(_, node)| node.kind == NodeKind::Window)
            .filter_map(|(id, _)| id.strip_prefix('w')?.parse().ok())
            .collect()
    }

    /// Adds a window's node, on top.
    pub fn add_window(&mut self, window: u64) {
        let id = window_node_id(window);
        let node = TreeNode::new(NodeKind::Window, Value::Null);
        self.nodes.insert(id.clone(), node);
        let root = self.root.clone();
        self.insert(&root, id, None);
    }

    /// Removes a window's node and what's drawn on top of it.
    pub fn remove_window(&mut self, window: u64) {
        let id = window_node_id(window);
        self.detach(&id);
        self.drop_subtree(&id);
    }

    /// Takes a window's node out, to put it in another workspace's tree.
    pub fn take_window(&mut self, window: u64) -> Option<Subtree> {
        self.take(&window_node_id(window))
    }

    /// Puts a window's node in, on top.
    pub fn put_window(&mut self, subtree: Subtree) {
        let root = self.root.clone();
        self.put(subtree, &root, None);
    }

    /// Takes a node and everything under it out, to put it in another workspace's tree.
    pub fn take(&mut self, id: &str) -> Option<Subtree> {
        if id == self.root || !self.nodes.contains_key(id) {
            return None;
        }
        self.detach(id);
        let mut taken = Vec::new();
        let mut pending = vec![id.to_owned()];
        while let Some(id) = pending.pop() {
            if let Some(node) = self.nodes.remove(&id) {
                pending.extend(node.children.iter().cloned());
                taken.push((id, node));
            }
        }
        Some(Subtree(taken))
    }

    /// Puts a node that was taken out in, in `parent`, last or before `before`.
    pub fn put(&mut self, subtree: Subtree, parent: &str, before: Option<&str>) {
        let Some((id, _)) = subtree.0.first() else {
            return;
        };
        let id = id.clone();
        self.nodes.extend(subtree.0);
        self.insert(parent, id, before);
    }

    /// Ids of a node and of everything under it.
    pub fn subtree_ids(&self, id: &str) -> Vec<String> {
        let mut ids = Vec::new();
        self.visit(id, &mut |id, _| ids.push(id.to_owned()));
        ids
    }

    /// Starts over for a new layout: the layout's own nodes are gone and the windows are in the
    /// order given, from the bottom to the top.
    pub fn reset(&mut self, windows: impl IntoIterator<Item = u64>) {
        let root = TreeNode::new(NodeKind::Root, Value::Null);
        self.nodes.clear();
        self.nodes.insert(self.root.clone(), root);
        for window in windows {
            self.add_window(window);
        }
    }

    /// Carries out one op from the layout.
    pub fn apply(&mut self, op: TreeOp) {
        match op {
            TreeOp::Create { id, kind, props } => {
                let Some(kind) = NodeKind::parse(&kind) else {
                    warn!("the layout creates a node of unknown kind {kind:?}");
                    return;
                };
                self.nodes.insert(id, TreeNode::new(kind, props));
            }
            TreeOp::Set { id, props } => {
                let Some(node) = self.nodes.get_mut(&id) else {
                    return;
                };
                if let (Some(old), Value::Object(new)) = (node.props.as_object_mut(), props) {
                    old.extend(new);
                }
            }
            TreeOp::Append {
                parent,
                child,
                before,
            } => {
                if !self.nodes.contains_key(&parent) || !self.nodes.contains_key(&child) {
                    return;
                }
                if child == self.root || self.is_inside(&parent, &child) {
                    return;
                }
                self.detach(&child);
                self.insert(&parent, child, before.as_deref());
            }
            TreeOp::Remove { id } => {
                let Some(node) = self.nodes.get(&id) else {
                    return;
                };
                // Windows can't be removed, only moved.
                if id == self.root || node.kind == NodeKind::Window {
                    return;
                }
                self.detach(&id);
                self.drop_subtree(&id);
            }
        }
    }

    /// Whether `id` is `ancestor` or inside it.
    fn is_inside(&self, id: &str, ancestor: &str) -> bool {
        let mut at = Some(id);
        while let Some(current) = at {
            if current == ancestor {
                return true;
            }
            at = self.nodes.get(current).and_then(|n| n.parent.as_deref());
        }
        false
    }

    fn detach(&mut self, id: &str) {
        let Some(parent) = self.nodes.get_mut(id).and_then(|node| node.parent.take()) else {
            return;
        };
        if let Some(parent) = self.nodes.get_mut(&parent) {
            parent.children.retain(|child| child != id);
        }
    }

    fn insert(&mut self, parent: &str, child: String, before: Option<&str>) {
        let Some(parent_node) = self.nodes.get_mut(parent) else {
            return;
        };
        let at = before
            .and_then(|before| parent_node.children.iter().position(|c| c == before))
            .unwrap_or(parent_node.children.len());
        parent_node.children.insert(at, child.clone());
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent.to_owned());
        }
    }

    /// Drops a node that's no longer in its parent, with its descendants. Windows in there stay,
    /// on top of the root.
    fn drop_subtree(&mut self, id: &str) {
        let Some(node) = self.nodes.remove(id) else {
            return;
        };
        for child in node.children {
            if self
                .nodes
                .get(&child)
                .is_some_and(|node| node.kind == NodeKind::Window)
            {
                if let Some(node) = self.nodes.get_mut(&child) {
                    node.parent = None;
                }
                let root = self.root.clone();
                self.insert(&root, child, None);
            } else {
                self.drop_subtree(&child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn create(id: &str, props: Value) -> TreeOp {
        TreeOp::Create {
            id: id.to_owned(),
            kind: String::from("box"),
            props,
        }
    }

    fn append(parent: &str, child: &str, before: Option<&str>) -> TreeOp {
        TreeOp::Append {
            parent: parent.to_owned(),
            child: child.to_owned(),
            before: before.map(str::to_owned),
        }
    }

    fn children(tree: &Tree, id: &str) -> Vec<String> {
        tree.node(id).unwrap().children.clone()
    }

    #[test]
    fn windows_go_on_top_and_stack_by_the_tree() {
        let mut tree = Tree::new(2);
        tree.add_window(7);
        tree.add_window(8);
        assert_eq!(tree.window_order(), [7, 8]);

        tree.apply(append("r2", "w7", None));
        assert_eq!(tree.window_order(), [8, 7]);
    }

    #[test]
    fn nodes_are_created_moved_and_removed() {
        let mut tree = Tree::new(2);
        tree.add_window(7);
        tree.apply(create("n1", json!({ "opacity": 1 })));
        tree.apply(append("r2", "n1", Some("w7")));
        assert_eq!(children(&tree, "r2"), ["n1", "w7"]);

        tree.apply(TreeOp::Set {
            id: String::from("n1"),
            props: json!({ "opacity": 0.5, "clip": true }),
        });
        assert_eq!(
            tree.node("n1").unwrap().props,
            json!({ "opacity": 0.5, "clip": true })
        );

        tree.apply(TreeOp::Remove {
            id: String::from("n1"),
        });
        assert!(tree.node("n1").is_none());
        assert_eq!(children(&tree, "r2"), ["w7"]);
    }

    #[test]
    fn removing_a_node_drops_its_subtree_but_not_windows() {
        let mut tree = Tree::new(2);
        tree.add_window(7);
        tree.apply(create("n1", json!({})));
        tree.apply(create("n2", json!({})));
        tree.apply(append("r2", "n1", None));
        tree.apply(append("n1", "n2", None));
        tree.apply(append("n1", "w7", None));

        tree.apply(TreeOp::Remove {
            id: String::from("n1"),
        });
        assert!(tree.node("n1").is_none());
        assert!(tree.node("n2").is_none());
        assert_eq!(children(&tree, "r2"), ["w7"]);
    }

    #[test]
    fn windows_cannot_be_removed() {
        let mut tree = Tree::new(2);
        tree.add_window(7);
        tree.apply(TreeOp::Remove {
            id: String::from("w7"),
        });
        assert_eq!(tree.window_order(), [7]);
    }

    #[test]
    fn a_window_moves_with_what_is_drawn_on_it() {
        let mut from = Tree::new(2);
        let mut to = Tree::new(3);
        from.add_window(7);
        from.apply(create("n1", json!({})));
        from.apply(append("w7", "n1", None));

        let subtree = from.take_window(7).unwrap();
        assert!(from.node("w7").is_none());
        assert!(from.node("n1").is_none());
        assert!(from.window_order().is_empty());

        to.put_window(subtree);
        assert_eq!(to.window_order(), [7]);
        assert_eq!(children(&to, "w7"), ["n1"]);
    }

    #[test]
    fn a_node_cannot_go_inside_itself() {
        let mut tree = Tree::new(2);
        tree.apply(create("n1", json!({})));
        tree.apply(create("n2", json!({})));
        tree.apply(append("r2", "n1", None));
        tree.apply(append("n1", "n2", None));
        tree.apply(append("n2", "n1", None));
        assert_eq!(children(&tree, "r2"), ["n1"]);
        assert_eq!(children(&tree, "n1"), ["n2"]);
    }

    #[test]
    fn reset_keeps_only_windows() {
        let mut tree = Tree::new(2);
        tree.add_window(7);
        tree.apply(create("n1", json!({})));
        tree.apply(append("r2", "n1", None));

        tree.reset([8, 7]);
        assert!(tree.node("n1").is_none());
        assert_eq!(tree.window_order(), [8, 7]);
    }
}
