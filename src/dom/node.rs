use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::commands::command::NodeId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomNode {
    pub id: NodeId,
    pub tag: String,
    #[serde(default)]
    pub attrs: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub children: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DomSnapshot {
    #[serde(default)]
    pub nodes: Vec<DomNode>,
}
