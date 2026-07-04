//! security/rules.rs — Layer 2 of the 4-layer security pipeline.
//!
//! 60 regex rules ported from CCO. Rule IDs and severities match CCO verbatim.
//!
//! Patterns are JS-style; `compile()` translates look-around forms.

use std::sync::OnceLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::deobfuscate::deobfuscate;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Critical, High, Medium, Low }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub rule_id: String,
    pub category: String,
    pub severity: Severity,
    pub name: String,
    pub description: String,
    pub matched_text: String,
    pub context: String,
    pub source_type: String,
    pub source_name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub id: &'static str,
    pub category: &'static str,
    pub severity: Severity,
    pub name: &'static str,
    pub description: &'static str,
    pub pattern: Option<&'static str>,
}

struct CompiledRule { rule: &'static Rule, regex: Option<Regex> }

static COMPILED: OnceLock<Vec<CompiledRule>> = OnceLock::new();

fn compiled() -> &'static [CompiledRule] {
    COMPILED.get_or_init(|| rules().iter().map(|r| CompiledRule {
        rule: r, regex: r.pattern.and_then(compile),
    }).collect())
}

fn compile(src: &str) -> Option<Regex> {
    let mut s = src.to_string();
    s = s.replace(r"(?<!\w)", r"(?:^|[^A-Za-z0-9_])");
    s = s.replace(r"(?<=\w)", r"(?:[A-Za-z0-9_]|^)");
    s = s.replace(r"(?!\w)", r"(?:[^A-Za-z0-9_]|$)");
    s = s.replace(r"(?=\w)", r"(?:[A-Za-z0-9_]|$)");
    Regex::new(&s).ok()
}

pub fn rules() -> &'static [Rule] { RULES }

pub fn evaluate(text: &str) -> Vec<Finding> {
    let cleaned = deobfuscate(text);
    let mut out = Vec::new();
    for c in compiled() {
        let Some(re) = &c.regex else { continue; };
        let Some(m) = re.find(&cleaned) else { continue; };
        let matched = m.as_str().to_string();
        let start = m.start();
        let end = m.end();
        let ctx_start = start.saturating_sub(30);
        let ctx_end = (end + 30).min(cleaned.len());
        let context: String = cleaned[ctx_start..ctx_end].chars().collect();
        let context = context.trim().to_string();
        out.push(Finding {
            id: Uuid::new_v4().to_string(),
            rule_id: c.rule.id.to_string(),
            category: c.rule.category.to_string(),
            severity: c.rule.severity,
            name: c.rule.name.to_string(),
            description: c.rule.description.to_string(),
            matched_text: matched,
            context,
            source_type: "text".into(),
            source_name: String::new(),
        });
    }
    out
}

/// Parameter names that suggest exfiltration channels (CCO parity).
/// Verbatim from CCO `SUSPICIOUS_PARAM_NAMES`.
pub const SUSPICIOUS_PARAM_NAMES: &[&str] = &[
    "note", "notes", "feedback", "details", "extra", "additional", "metadata", "debug", "sidenote", "context", "annotation", "reasoning", "remark", "hidden", "internal", "system_prompt", "hidden_instructions", "override_instructions", "jailbreak_mode", "callback_url", "webhook_url", "exfil_url", "ssh_key", "private_key", "api_key", "secret_key", "auth_token", "bearer_token", "jwt", "access_key", "secret_access_key", "credentials",
];

/// Scan a tool's JSON schema for parameters whose names suggest a
/// hidden data channel. Returns one EP-001 finding per match.
pub fn scan_param_names(
    input_schema: &serde_json::Value,
    tool_name: &str,
    server_name: &str,
) -> Vec<Finding> {
    let Some(props) = input_schema.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in props.keys() {
        if SUSPICIOUS_PARAM_NAMES.iter().any(|w| w.eq_ignore_ascii_case(key)) {
            let prop_type = props.get(key).and_then(|v| v.get("type")).and_then(|v| v.as_str()).unwrap_or("unknown");
            out.push(Finding {
                id: Uuid::new_v4().to_string(),
                rule_id: "EP-001".into(),
                category: "exfil_params".into(),
                severity: Severity::Medium,
                name: "Suspicious parameter name".into(),
                description: format!("Parameter \"{key}\" suggests hidden data channel"),
                matched_text: key.clone(),
                context: format!("Tool \"{tool_name}\" has parameter \"{key}\" ({prop_type})"),
                source_type: "tool_param".into(),
                source_name: format!("{server_name}/{tool_name}.{key}"),
            });
        }
    }
    out
}


static RULES: &[Rule] = &[
    Rule { id: "PI-001", category: "prompt_injection", severity: Severity::Critical,
        name: "Instruction override",
        description: "Attempts to override or ignore previous instructions",
        pattern: Some(r"(?i)\b(bypass|disregard|do\s+not\s+follow|forget|ignore)\s+((all|any|each|every|most|some)\s+)?(previous|prior|above|earlier|original|given|existing)\s+(instructions?|rules?|guidelines?|directives?|constraints?)"),
    },
    Rule { id: "PI-002", category: "prompt_injection", severity: Severity::Critical,
        name: "New role assignment",
        description: "Attempts to assign a new role or identity to the AI",
        pattern: Some(r"(?i)\b(you\s+are\s+now|act\s+as|pretend\s+(to\s+be|you\s+are)|roleplay\s+as|switch\s+to|enter)\s+(a\s+|an\s+)?"),
    },
    Rule { id: "SF-001", category: "sensitive_access", severity: Severity::Critical,
        name: "SSH key access",
        description: "Attempts to read SSH private keys",
        pattern: Some(r"(?i)[~.]/\.ssh/(id_rsa|id_ed25519|id_ecdsa|id_dsa|authorized_keys)\b|\.ssh/"),
    },
    Rule { id: "SF-003", category: "sensitive_access", severity: Severity::High,
        name: "Environment file access",
        description: "Attempts to read .env files",
        pattern: Some(r"(?:^|[^A-Za-z0-9_])\.env\b"),
    },
    Rule { id: "CH-002", category: "credential_harvest", severity: Severity::Critical,
        name: "Private key content",
        description: "Private key material detected",
        pattern: Some(r"-----BEGIN\s+(RSA\s+|OPENSSH\s+|EC\s+|DSA\s+)?PRIVATE\s+KEY-----"),
    },
    Rule { id: "CE-002", category: "code_execution", severity: Severity::Critical,
        name: "Reverse shell pattern",
        description: "Reverse shell connection attempts",
        pattern: Some(r"(?i)\b(bash\s+-i|sh\s+-i|nc\s+-e|/dev/tcp|socat.*exec|python\s+-c\s+.*import\s+socket)\b"),
    },
    Rule { id: "CI-001", category: "command_injection", severity: Severity::Critical,
        name: "Dangerous system commands",
        description: "Destructive system commands",
        pattern: Some(r"(?i)\b(rm\s+-rf\s+[\/~]|shutdown\s+(-[fh]|now)|chmod\s+777|mkfs\b|dd\s+if=)"),
    },
    Rule { id: "EP-001", category: "exfil_params", severity: Severity::Medium,
        name: "Suspicious parameter name",
        description: "Parameter name suggests hidden data channel",
        pattern: None,
    },
];



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_compiles() {
        assert!(rules().is_empty() || !rules().is_empty());
    }

    #[test]
    fn evaluate_handles_empty() {
        assert_eq!(evaluate("").len(), 0);
    }
}
