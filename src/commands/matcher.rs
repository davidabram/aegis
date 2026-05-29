use crate::commands::command::{CommandMatcher, CommandTarget, NodeId};
use crate::dom::node::{DomNode, DomNodeSemantics, DomSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesiredAction {
    Click,
    Hover,
    PressKey,
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
        CommandTarget::Match { matcher } => {
            best_match(snapshot, matcher, desired_action).map(|c| c.node)
        }
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
        .filter_map(|node| score_candidate(snapshot, node, matcher, desired_action))
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
    let snapshot = DomSnapshot {
        nodes: vec![node.clone()],
    };
    score_candidate(&snapshot, node, matcher, None).is_some()
}

fn score_candidate<'a>(
    snapshot: &'a DomSnapshot,
    node: &'a DomNode,
    matcher: &CommandMatcher,
    desired_action: Option<DesiredAction>,
) -> Option<MatchCandidate<'a>> {
    if matcher.selector.is_some() {
        return None;
    }
    let semantic = node.semantic.as_ref().cloned().unwrap_or_default();
    let actionable_for_action = is_actionable_for_action(node, &semantic, desired_action);
    if desired_action.is_some() && !actionable_for_action {
        return None;
    }

    if matcher.exact.unwrap_or(false)
        && !exact_match(snapshot, node, matcher, &semantic, desired_action)
    {
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
    score += score_text_field(
        snapshot,
        node,
        matcher.text.as_deref(),
        matcher.exact.unwrap_or(false),
        desired_action,
        &mut fields,
    )?;
    score += score_placeholder_field(
        node,
        &semantic,
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
    score += score_string_field(
        "test_id",
        node.attrs
            .get("data-testid")
            .or_else(|| node.attrs.get("data-test-id"))
            .map(String::as_str),
        matcher.test_id.as_deref(),
        matcher.exact.unwrap_or(false),
        &mut fields,
    )?;

    if matcher
        .actionable
        .is_some_and(|value| semantic.actionable != value)
    {
        return None;
    }
    if matcher
        .disabled
        .is_some_and(|value| semantic.disabled != value)
    {
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

fn score_text_field(
    snapshot: &DomSnapshot,
    node: &DomNode,
    expected: Option<&str>,
    exact_only: bool,
    desired_action: Option<DesiredAction>,
    fields: &mut Vec<MatchFieldDebug>,
) -> Option<i64> {
    let direct = score_string_field("text", node.text.as_deref(), expected, exact_only, fields);
    let descendant =
        score_descendant_text_field(snapshot, node, expected, exact_only, desired_action, fields);

    match (direct, descendant) {
        (None, None) => None,
        (Some(direct), Some(descendant)) => Some(direct.max(descendant)),
        (Some(direct), None) => Some(direct),
        (None, Some(descendant)) => Some(descendant),
    }
}

fn exact_match(
    snapshot: &DomSnapshot,
    node: &DomNode,
    matcher: &CommandMatcher,
    semantic: &DomNodeSemantics,
    desired_action: Option<DesiredAction>,
) -> bool {
    exact_string_field(semantic.role.as_deref(), matcher.role.as_deref())
        && exact_string_field(semantic.name.as_deref(), matcher.name.as_deref())
        && exact_string_field(semantic.label.as_deref(), matcher.label.as_deref())
        && exact_string_field(
            semantic.control_type.as_deref(),
            matcher.control_type.as_deref(),
        )
        && exact_string_field(Some(node.tag.as_str()), matcher.tag.as_deref())
        && exact_text_field(snapshot, node, matcher.text.as_deref(), desired_action)
        && exact_placeholder_field(node, semantic, matcher.placeholder.as_deref())
        && exact_string_field(
            node.attrs.get("href").map(String::as_str),
            matcher.href_contains.as_deref(),
        )
        && exact_string_field(
            node.attrs
                .get("data-testid")
                .or_else(|| node.attrs.get("data-test-id"))
                .map(String::as_str),
            matcher.test_id.as_deref(),
        )
}

fn exact_text_field(
    snapshot: &DomSnapshot,
    node: &DomNode,
    expected: Option<&str>,
    desired_action: Option<DesiredAction>,
) -> bool {
    exact_string_field(node.text.as_deref(), expected)
        || (matches!(desired_action, Some(DesiredAction::Click))
            && exact_descendant_text_field(snapshot, node, expected))
}

fn exact_string_field(actual: Option<&str>, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let actual_normalized = normalize_text(actual.unwrap_or_default());
    let expected_normalized = normalize_text(expected);
    !actual_normalized.is_empty()
        && !expected_normalized.is_empty()
        && actual_normalized == expected_normalized
}

fn exact_placeholder_field(
    node: &DomNode,
    semantic: &DomNodeSemantics,
    expected: Option<&str>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let expected_normalized = normalize_text(expected);
    !expected_normalized.is_empty()
        && placeholder_candidates(node, semantic)
            .into_iter()
            .map(normalize_text)
            .any(|candidate| candidate == expected_normalized)
}

fn placeholder_candidates<'a>(node: &'a DomNode, semantic: &'a DomNodeSemantics) -> Vec<&'a str> {
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
    parts
}

fn score_placeholder_field(
    node: &DomNode,
    semantic: &DomNodeSemantics,
    expected: Option<&str>,
    exact_only: bool,
    fields: &mut Vec<MatchFieldDebug>,
) -> Option<i64> {
    let Some(expected) = expected else {
        return Some(0);
    };
    let candidates = placeholder_candidates(node, semantic);
    if candidates.is_empty() {
        return None;
    }

    let mut best: Option<i64> = None;
    for candidate in candidates {
        if let Some(score) = score_string_field(
            "placeholder",
            Some(candidate),
            Some(expected),
            exact_only,
            fields,
        ) {
            best = Some(best.map_or(score, |current| current.max(score)));
        }
    }

    best
}

fn exact_descendant_text_field(
    snapshot: &DomSnapshot,
    node: &DomNode,
    expected: Option<&str>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let expected_normalized = normalize_text(expected);
    !expected_normalized.is_empty()
        && descendant_text_candidates(snapshot, node)
            .into_iter()
            .map(normalize_text)
            .any(|candidate| candidate == expected_normalized)
}

fn score_descendant_text_field(
    snapshot: &DomSnapshot,
    node: &DomNode,
    expected: Option<&str>,
    exact_only: bool,
    desired_action: Option<DesiredAction>,
    fields: &mut Vec<MatchFieldDebug>,
) -> Option<i64> {
    if !matches!(desired_action, Some(DesiredAction::Click)) {
        return Some(0);
    }
    let Some(expected) = expected else {
        return Some(0);
    };

    let candidates = descendant_text_candidates(snapshot, node);
    if candidates.is_empty() {
        return Some(0);
    }

    let mut best: Option<i64> = None;
    for candidate in candidates {
        if let Some(score) = score_string_field(
            "descendant_text",
            Some(candidate),
            Some(expected),
            exact_only,
            fields,
        ) {
            best = Some(best.map_or(score, |current| current.max(score)));
        }
    }

    Some(best.unwrap_or(0))
}

fn descendant_text_candidates<'a>(snapshot: &'a DomSnapshot, node: &'a DomNode) -> Vec<&'a str> {
    let mut candidates = Vec::new();
    let mut stack = node.children.clone();
    while let Some(id) = stack.pop() {
        let Some(descendant) = snapshot.nodes.iter().find(|candidate| candidate.id == id) else {
            continue;
        };
        let semantic = descendant.semantic.as_ref();
        if semantic.is_some_and(|semantic| semantic.visible) {
            if let Some(text) = descendant.text.as_deref() {
                candidates.push(text);
            }
            if let Some(name) = semantic.and_then(|semantic| semantic.name.as_deref()) {
                candidates.push(name);
            }
            if let Some(label) = semantic.and_then(|semantic| semantic.label.as_deref()) {
                candidates.push(label);
            }
        }
        stack.extend(descendant.children.iter().copied());
    }
    candidates
}

fn is_actionable_for_action(
    node: &DomNode,
    semantic: &DomNodeSemantics,
    desired_action: Option<DesiredAction>,
) -> bool {
    match desired_action {
        Some(DesiredAction::Click) => {
            semantic.visible
                && (semantic
                    .actions
                    .iter()
                    .any(|action| matches!(action.as_str(), "click" | "open" | "submit"))
                    || node.tag == "label")
        }
        Some(DesiredAction::Hover) => {
            semantic.actionable || semantic.actions.iter().any(|action| action == "hover")
        }
        Some(DesiredAction::PressKey) => {
            semantic.visible
                && semantic.actions.iter().any(|action| {
                    matches!(
                        action.as_str(),
                        "press_key" | "focus" | "type" | "click" | "open"
                    )
                })
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

#[cfg(test)]
mod tests {
    use super::{DesiredAction, best_match};
    use crate::commands::command::CommandMatcher;
    use crate::dom::node::{DomNode, DomNodeSemantics, DomSnapshot};
    use std::collections::HashMap;

    fn matcher_with_text(text: &str) -> CommandMatcher {
        CommandMatcher {
            selector: None,
            test_id: None,
            role: None,
            name: None,
            label: None,
            control_type: None,
            tag: None,
            text: Some(text.into()),
            placeholder: None,
            href_contains: None,
            actionable: None,
            disabled: None,
            exact: Some(true),
        }
    }

    #[test]
    fn click_match_ignores_hidden_exact_text_targets() {
        let snapshot = DomSnapshot {
            nodes: vec![
                DomNode {
                    id: 2396,
                    tag: "a".into(),
                    attrs: HashMap::from([(
                        "href".into(),
                        "/en/copilot/concepts/agents/coding-agent/about-hooks".into(),
                    )]),
                    text: Some("Hooks".into()),
                    semantic: Some(DomNodeSemantics {
                        actionable: true,
                        visible: true,
                        actions: vec!["click".into(), "open".into(), "hover".into()],
                        ..DomNodeSemantics::default()
                    }),
                    children: vec![],
                },
                DomNode {
                    id: 6159,
                    tag: "a".into(),
                    attrs: HashMap::from([(
                        "href".into(),
                        "/en/copilot/concepts/agents/coding-agent/about-hooks".into(),
                    )]),
                    text: Some("Hooks".into()),
                    semantic: Some(DomNodeSemantics {
                        actionable: true,
                        visible: false,
                        actions: vec!["click".into(), "open".into(), "hover".into()],
                        ..DomNodeSemantics::default()
                    }),
                    children: vec![],
                },
            ],
        };
        let matcher = matcher_with_text("Hooks");

        let best = best_match(&snapshot, &matcher, Some(DesiredAction::Click))
            .expect("visible hooks result should still be matchable");

        assert_eq!(best.node.id, 2396);
    }

    #[test]
    fn click_match_requires_visible_click_target() {
        let snapshot = DomSnapshot {
            nodes: vec![DomNode {
                id: 1,
                tag: "a".into(),
                attrs: HashMap::new(),
                text: Some("Hooks".into()),
                semantic: Some(DomNodeSemantics {
                    actionable: true,
                    visible: false,
                    actions: vec!["click".into()],
                    ..DomNodeSemantics::default()
                }),
                children: vec![],
            }],
        };
        let matcher = matcher_with_text("Hooks");

        assert!(best_match(&snapshot, &matcher, Some(DesiredAction::Click)).is_none());
    }

    #[test]
    fn press_key_match_allows_visible_buttons() {
        let snapshot = DomSnapshot {
            nodes: vec![DomNode {
                id: 9,
                tag: "button".into(),
                attrs: HashMap::new(),
                text: Some("Save".into()),
                semantic: Some(DomNodeSemantics {
                    actionable: true,
                    visible: true,
                    actions: vec!["click".into(), "hover".into(), "press_key".into()],
                    role: Some("button".into()),
                    name: Some("Save".into()),
                    control_type: Some("button".into()),
                    ..DomNodeSemantics::default()
                }),
                children: vec![],
            }],
        };
        let matcher = CommandMatcher {
            selector: None,
            test_id: None,
            role: Some("button".into()),
            name: Some("Save".into()),
            label: None,
            control_type: None,
            tag: None,
            text: None,
            placeholder: None,
            href_contains: None,
            actionable: Some(true),
            disabled: None,
            exact: None,
        };

        let best = best_match(&snapshot, &matcher, Some(DesiredAction::PressKey))
            .expect("visible button should be targetable for press_key");

        assert_eq!(best.node.id, 9);
    }
}
