//! Plan 10 — critical-finding desktop notification with hash dedup.
//!
//! `NotificationState` remembers the IDs of findings it has already
//! toasted for. `fire_if_new` returns `true` the first time a given
//! finding hash is seen, `false` thereafter — so an editor that saves
//! the same poisoned config five times in a row only spams the user
//! once.
//!
//! The actual desktop notification is delivered through
//! `tauri-plugin-notification`. We don't take a hard dep on a `App`
//! context in this module so the dedup logic is unit-testable without
//! Tauri running.

use std::collections::HashSet;

use crate::security::rules::{Finding, Severity};

/// Tracks which findings the user has already been notified about.
/// The state is keyed on the finding's UUID `id` (which is stable
/// across rescans of the same source/rule combo).
#[derive(Debug, Default, Clone)]
pub struct NotificationState {
    /// IDs of findings we've already fired a notification for.
    pub seen: HashSet<String>,
}

impl NotificationState {
    pub fn new() -> Self {
        Self { seen: HashSet::new() }
    }

    /// Number of findings already announced. Useful for the tray
    /// glance UI ("N notifications raised this session").
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// True when no findings have been announced yet. The tray uses
    /// this to render a "clean" pill in the glance view.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// Decide whether `finding` warrants a desktop notification.
/// Returns `true` when this is the first time we've seen the
/// finding's id; updates `state` on the way out.
///
/// "Critical" is the only severity that fires today. The user can
/// already tolerate lower-severity findings; spamming them for
/// "medium" or "low" defeats the purpose of a notification.
pub fn fire_if_new(
    state: &mut NotificationState,
    finding: &Finding,
) -> (bool, NotificationOutcome) {
    if !matches!(finding.severity, Severity::Critical) {
        return (false, NotificationOutcome::SkippedNonCritical);
    }

    if state.seen.contains(&finding.id) {
        return (false, NotificationOutcome::AlreadyFired);
    }
    state.seen.insert(finding.id.clone());

    (true, NotificationOutcome::Fired)
}

/// Synthetic result of a `fire_if_new` decision. The real send goes
/// through the plugin; this enum is for tests and logging only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationOutcome {
    /// We didn't fire — the finding isn't critical.
    SkippedNonCritical,
    /// We didn't fire — we've already fired for this finding id.
    AlreadyFired,
    /// We fired (or would fire, in tests) — first time seeing this id.
    Fired,
}

/// Build the human-readable title + body for a finding. Centralised
/// here so the tray glance, the desktop notification, and any future
/// surfaces share the exact same text.
pub fn format_notification(finding: &Finding) -> (String, String) {
    let title = format!("Ward: critical finding in {}", finding.source_name);
    let body = format!(
        "{} — {} (rule {}): {}",
        finding.rule_id,
        finding.name,
        finding.rule_id,
        truncate(&finding.matched_text, 120)
    );
    (title, body)
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: &str, severity: Severity) -> Finding {
        Finding {
            id: id.to_string(),
            rule_id: "PI-001".to_string(),
            category: "prompt_injection".to_string(),
            severity,
            name: "Test finding".to_string(),
            description: "Attempts to override or ignore previous instructions".to_string(),
            matched_text: "ignore previous instructions".to_string(),
            context: "ignore previous instructions".to_string(),
            source_type: "text".to_string(),
            source_name: "evil-mcp".to_string(),
        }
    }

    #[test]
    fn fires_on_first_critical() {
        let mut state = NotificationState::new();
        let (fired, outcome) = fire_if_new(&mut state, &finding("a", Severity::Critical));
        assert!(fired);
        assert_eq!(outcome, NotificationOutcome::Fired);
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn does_not_refire_same_finding() {
        let mut state = NotificationState::new();
        let _ = fire_if_new(&mut state, &finding("a", Severity::Critical));
        let (fired, outcome) = fire_if_new(&mut state, &finding("a", Severity::Critical));
        assert!(!fired);
        assert_eq!(outcome, NotificationOutcome::AlreadyFired);
        assert_eq!(state.len(), 1, "state must not grow on second hit");
    }

    #[test]
    fn fires_again_on_new_critical() {
        let mut state = NotificationState::new();
        let _ = fire_if_new(&mut state, &finding("a", Severity::Critical));
        let (fired, _) = fire_if_new(&mut state, &finding("b", Severity::Critical));
        assert!(fired);
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn skips_non_critical() {
        let mut state = NotificationState::new();
        for sev in [Severity::High, Severity::Medium, Severity::Low] {
            let (fired, outcome) = fire_if_new(&mut state, &finding("x", sev));
            assert!(!fired);
            assert_eq!(outcome, NotificationOutcome::SkippedNonCritical);
        }
        assert!(state.is_empty());
    }

    #[test]
    fn empty_state_is_empty() {
        let state = NotificationState::new();
        assert!(state.is_empty());
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn state_remembers_after_populating() {
        let mut state = NotificationState::new();
        let _ = fire_if_new(&mut state, &finding("a", Severity::Critical));
        assert!(!state.is_empty());
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn format_notification_includes_finding_fields() {
        let f = finding("a", Severity::Critical);
        let (title, body) = format_notification(&f);
        assert!(title.contains("evil-mcp"), "title should name the source");
        assert!(title.contains("critical"), "title should flag severity");
        assert!(body.contains("PI-001"), "body should include the rule id");
        assert!(body.contains("ignore previous instructions"));
    }

    #[test]
    fn truncate_caps_long_strings() {
        let long = "a".repeat(500);
        let t = truncate(&long, 100);
        assert!(t.chars().count() <= 101, "expected ≤ 100 chars + ellipsis");
        assert!(t.ends_with('…'));
    }

    #[test]
    fn truncate_passes_through_short_strings() {
        let s = "hello";
        assert_eq!(truncate(s, 100), "hello");
    }
}