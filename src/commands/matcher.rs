use crate::commands::command::{CommandMatcher, CommandTarget, NodeId};
use crate::dom::node::{DomNode, DomNodeSemantics, DomSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesiredAction {
    Click,
    Hover,
    Type,
}

#[derive(Debug, Clone)]
pub struct MatchFieldDebug {
    pub field: &'static str,
    pub exact: bool,
    pub score: i64,
}

#[derive(Debug, Clone)]
pub struct MatchCandidate<'a> {
    pub node: &'a DomNode,
    pub score: i64,
    pub actionable_for_action: bool,
    pub fields: Vec<MatchFieldDebug>,
}

pub fn resolve_command_target<'a>(
    snapshot: &'a DomSnapshot,
    target: &CommandTarget,
    desired_action: Option<DesiredAction>,
) -> Option<&'a DomNode> {
    match target {
        CommandTarget::Id { id } => snapshot.nodes.iter().find(|node| node.id == *id),
        CommandTarget::Match { matcher } => best_match(snapshot, matcher, desired_action).map(|c| c.node),
    }
}

pub fn best_match<'a>(
    snapshot: &'a DomSnapshot,
    matcher: &CommandMatcher,
    desired_action: Option<DesiredAction>,
) -> Option<MatchCandidate<'a>> {
    let mut candidates = snapshot
        .nodes
        .iter()
        .filter_map(|node| score_candidate(node, matcher, desired_action))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| right.actionable_for_action.cmp(&left.actionable_for_action))
            .then_with(|| left.node.id.cmp(&right.node.id))
    });
    candidates.into_iter().next()
}

pub fn matches_command_matcher(node: &DomNode, matcher: &CommandMatcher) -> bool {
    score_candidate(node, matcher, None).is_some()
}

fn score_candidate<'a>(
    node: &'a DomNode,
    matcher: &CommandMatcher,
    desired_action: Option<DesiredAction>,
) -> Option<MatchCandidate<'a>> {
    let semantic = node.semantic.as_ref().cloned().unwrap_or_default();
    let actionable_for_action = is_actionable_for_action(node, &semantic, desired_action);
    if desired_action.is_some() && !actionable_for_action {
        return None;
    }

    let mut score = 0_i64;
    let mut fields = Vec::new();

    score += score_string_field(
        "role",
        semantic.role.as_deref(),
        matcher.role.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "name",
        semantic.name.as_deref(),
        matcher.name.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "label",
        semantic.label.as_deref(),
        matcher.label.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "control_type",
        semantic.control_type.as_deref(),
        matcher.control_type.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "tag",
        Some(node.tag.as_str()),
        matcher.tag.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "text",
        node.text.as_deref(),
        matcher.text.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "placeholder",
        placeholder_haystack(node, &semantic).as_deref(),
        matcher.placeholder.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;
    score += score_string_field(
        "href_contains",
        node.attrs.get("href").map(String::as_str),
        matcher.href_contains.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;

    if matcher
        .actionable
        .is_some_and(|value| semantic.actionable != value)
    {
        return None;
    }
    if matcher.disabled.is_some_and(|value| semantic.disabled != value) {
        return None;
    }

    if actionable_for_action {
        score += 120;
    } else if semantic.actionable {
        score += 20;
    }

    Some(MatchCandidate {
        node,
        score,
        actionable_for_action,
        fields,
    })
}

fn score_string_field(
    field: &'static str,
    actual: Option<&str>,
    expected: Option<&str>,
    exact_only: bool,
    fields: &mut Vec<MatchFieldDebug>,
) -> Option<i64> {
    let Some(expected) = expected else {
        return Some(0);
    };
    let actual_normalized = normalize_text(actual.unwrap_or_default());
    let expected_normalized = normalize_text(expected);
    if expected_normalized.is_empty() {
        return Some(0);
    }
    if actual_normalized.is_empty() {
        return None;
    }
    if actual_normalized == expected_normalized {
        fields.push(MatchFieldDebug {
            field,
            exact: true,
            score: 120,
        });
        return Some(120);
    }
    if exact_only || !actual_normalized.contains(&expected_normalized) {
        return None;
    }
    fields.push(MatchFieldDebug {
        field,
        exact: false,
        score: 60,
    });
    Some(60)
}

fn placeholder_haystack(node: &DomNode, semantic: &DomNodeSemantics) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = node.attrs.get("placeholder") {
        parts.push(value.as_str());
    }
    if let Some(value) = node.attrs.get("aria-label") {
        parts.push(value.as_str());
    }
    if let Some(value) = semantic.name.as_deref() {
        parts.push(value);
    }
    if let Some(value) = semantic.label.as_deref() {
        parts.push(value);
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn is_actionable_for_action(
    node: &DomNode,
    semantic: &DomNodeSemantics,
    desired_action: Option<DesiredAction>,
) -> bool {
    match desired_action {
        Some(DesiredAction::Click) | Some(DesiredAction::Hover) => {
            semantic.actionable
                || semantic
                    .actions
                    .iter()
                    .any(|action| matches!(action.as_str(), "click" | "open" | "submit"))
                || node.tag == "label"
        }
        Some(DesiredAction::Type) => semantic.actions.iter().any(|action| action == "type"),
        None => true,
    }
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn target_id(target: &CommandTarget) -> Option<NodeId> {
    match target {
        CommandTarget::Id { id } => Some(*id),
        CommandTarget::Match { .. } => None,
    }
}
