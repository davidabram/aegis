use std::collections::HashMap;

use crate::commands::command::NodeId;
use crate::dom::diff::DomMutation;
use crate::dom::node::{DomNode, DomSnapshot};

#[derive(Debug, Clone, Default)]
pub struct DomTree {
    nodes: HashMap<NodeId, DomNode>,
}

impl DomTree {
    pub fn from_snapshot(snapshot: DomSnapshot) -> Self {
        let nodes = snapshot
            .nodes
            .into_iter()
            .map(|node| (node.id, node))
            .collect();
        Self { nodes }
    }

    pub fn replace_snapshot(&mut self, snapshot: DomSnapshot) {
        self.nodes = snapshot
            .nodes
            .into_iter()
            .map(|node| (node.id, node))
            .collect();
    }

    pub fn apply_mutations(&mut self, mutations: &[DomMutation]) {
        for mutation in mutations {
            match mutation {
                DomMutation::Upsert {
                    id,
                    tag,
                    attrs,
                    text,
                    children,
                } => {
                    self.nodes.insert(
                        *id,
                        DomNode {
                            id: *id,
                            tag: tag.clone(),
                            attrs: attrs.clone(),
                            text: text.clone(),
                            children: children.clone(),
                        },
                    );
                }
                DomMutation::Remove { id } => {
                    self.nodes.remove(id);
                    for parent in self.nodes.values_mut() {
                        parent.children.retain(|child| child != id);
                    }
                }
                DomMutation::SetText { id, text } => {
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.text = text.clone();
                    }
                }
                DomMutation::SetAttr { id, name, value } => {
                    if let Some(node) = self.nodes.get_mut(id) {
                        match value {
                            Some(value) => {
                                node.attrs.insert(name.clone(), value.clone());
                            }
                            None => {
                                node.attrs.remove(name);
                            }
                        }
                    }
                }
                DomMutation::SetChildren { id, children } => {
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.children = children.clone();
                    }
                }
            }
        }
    }

    pub fn snapshot(&self) -> DomSnapshot {
        let mut nodes: Vec<DomNode> = self.nodes.values().cloned().collect();
        nodes.sort_by_key(|node| node.id);
        DomSnapshot { nodes }
    }

    pub fn node(&self, id: NodeId) -> Option<&DomNode> {
        self.nodes.get(&id)
    }
}
