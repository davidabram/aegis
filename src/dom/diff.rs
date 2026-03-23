use serde::{Deserialize, Serialize};

use crate::commands::command::NodeId;
use crate::dom::node::{DomNode, DomSnapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DomMutation {
    Upsert(DomNode),
    Remove(NodeId),
    SetText {
        id: NodeId,
        text: Option<String>,
    },
    SetAttr {
        id: NodeId,
        name: String,
        value: Option<String>,
    },
    SetChildren {
        id: NodeId,
        children: Vec<NodeId>,
    },
}

pub fn diff_snapshots(previous: &DomSnapshot, next: &DomSnapshot) -> Vec<DomMutation> {
    let mut previous_nodes = previous
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<std::collections::HashMap<_, _>>();
    let next_nodes = next
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<std::collections::HashMap<_, _>>();

    let mut changes = Vec::new();

    for (&id, next_node) in &next_nodes {
        match previous_nodes.remove(&id) {
            None => changes.push(DomMutation::Upsert((*next_node).clone())),
            Some(previous_node) => {
                if previous_node.tag != next_node.tag {
                    changes.push(DomMutation::Upsert((*next_node).clone()));
                    continue;
                }

                if previous_node.text != next_node.text {
                    changes.push(DomMutation::SetText {
                        id,
                        text: next_node.text.clone(),
                    });
                }

                if previous_node.children != next_node.children {
                    changes.push(DomMutation::SetChildren {
                        id,
                        children: next_node.children.clone(),
                    });
                }

                for (name, value) in &next_node.attrs {
                    if previous_node.attrs.get(name) != Some(value) {
                        changes.push(DomMutation::SetAttr {
                            id,
                            name: name.clone(),
                            value: Some(value.clone()),
                        });
                    }
                }

                for name in previous_node.attrs.keys() {
                    if !next_node.attrs.contains_key(name) {
                        changes.push(DomMutation::SetAttr {
                            id,
                            name: name.clone(),
                            value: None,
                        });
                    }
                }
            }
        }
    }

    for id in previous_nodes.into_keys() {
        changes.push(DomMutation::Remove(id));
    }

    changes
}
