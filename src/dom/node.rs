use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::commands::command::NodeId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DomNodeSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_type: Option<String>,
    #[serde(default)]
    pub actionable: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomNode {
    pub id: NodeId,
    pub tag: String,
    #[serde(default)]
    pub attrs: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<DomNodeSemantics>,
    #[serde(default)]
    pub children: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DomSnapshot {
    #[serde(default)]
    pub nodes: Vec<DomNode>,
}
