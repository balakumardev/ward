//! Plan 29 — the curated Claude Code environment-variable catalog.
//!
//! The Settings mode exposes a secondary, search-driven list of the documented
//! environment variables users commonly set. Editing one of these writes a key
//! into the `env` object of `settings.json` (handled by the settings writer);
//! this module only supplies the metadata Ward shows next to each name.
//!
//! The list is a hand-curated table transcribed from Claude Code's published
//! environment-variable documentation (`https://code.claude.com/docs/en/env-vars`).
//! It is intentionally a *curated* subset — the full page lists well over a
//! hundred variables, many niche or provider-internal; this catalog covers the
//! authentication, feature-toggle, telemetry, behavior/limit, and config/proxy
//! variables a user is likely to reach for. Descriptions are factual and
//! one-line; no behavior is invented.

use serde::{Deserialize, Serialize};

/// One curated environment variable — its name, a factual one-line description,
/// and a grouping category for the Settings UI. Purely descriptive metadata;
/// the value a user sets is written to `settings.json`'s `env` object elsewhere.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvVarDef {
    /// The environment variable name, e.g. `"ANTHROPIC_API_KEY"`.
    pub name: String,
    /// One-line, factual description of what the variable does.
    pub description: String,
    /// Grouping label for the UI, e.g. `"Authentication & Provider"`.
    pub category: String,
}

/// Convenience constructor keeping the catalog table compact and readable.
fn e(name: &str, category: &str, description: &str) -> EnvVarDef {
    EnvVarDef {
        name: name.to_string(),
        description: description.to_string(),
        category: category.to_string(),
    }
}

// Category labels — kept as constants so the grouping is spelled once.
const CAT_AUTH: &str = "Authentication & Provider";
const CAT_TOGGLES: &str = "Feature Toggles";
const CAT_TELEMETRY: &str = "Telemetry & OpenTelemetry";
const CAT_BEHAVIOR: &str = "Behavior & Limits";
const CAT_CONFIG: &str = "Config, Paths & Proxy";

/// The curated environment-variable catalog Ward shows in Settings mode.
///
/// Ordered by category (auth → toggles → telemetry → behavior → config) so the
/// search-driven list groups sensibly. Every entry has a non-empty name,
/// description, and category; names are unique (guarded by tests).
pub fn env_catalog() -> Vec<EnvVarDef> {
    vec![
        // --- Authentication & Provider ---
        e(
            "ANTHROPIC_API_KEY",
            CAT_AUTH,
            "API key sent as the X-Api-Key header. When set, it is used instead of your Claude subscription login.",
        ),
        e(
            "ANTHROPIC_AUTH_TOKEN",
            CAT_AUTH,
            "Custom value for the Authorization header; the value you set is automatically prefixed with 'Bearer '.",
        ),
        e(
            "ANTHROPIC_BASE_URL",
            CAT_AUTH,
            "Override the API endpoint to route requests through a proxy or gateway.",
        ),
        e(
            "ANTHROPIC_MODEL",
            CAT_AUTH,
            "Model ID to use as the primary model, overriding the default selection.",
        ),
        e(
            "ANTHROPIC_SMALL_FAST_MODEL",
            CAT_AUTH,
            "Deprecated. Model ID of the Haiku-class model used for background tasks.",
        ),
        e(
            "ANTHROPIC_CUSTOM_HEADERS",
            CAT_AUTH,
            "Extra request headers in 'Name: Value' format, newline-separated for multiple headers.",
        ),
        e(
            "ANTHROPIC_BETAS",
            CAT_AUTH,
            "Comma-separated anthropic-beta header values to opt into Anthropic API betas; works with all auth methods.",
        ),
        e(
            "ANTHROPIC_VERTEX_PROJECT_ID",
            CAT_AUTH,
            "GCP project ID used for Google Vertex AI requests.",
        ),
        e(
            "CLAUDE_CODE_USE_BEDROCK",
            CAT_AUTH,
            "Set to 1 to route model requests through Amazon Bedrock instead of the Anthropic API.",
        ),
        e(
            "CLAUDE_CODE_USE_VERTEX",
            CAT_AUTH,
            "Set to 1 to route model requests through Google Vertex AI instead of the Anthropic API.",
        ),
        e(
            "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
            CAT_AUTH,
            "Set to 1 to skip AWS authentication for Bedrock, e.g. when routing through an LLM gateway.",
        ),
        e(
            "CLAUDE_CODE_SKIP_VERTEX_AUTH",
            CAT_AUTH,
            "Set to 1 to skip Google authentication for Vertex, e.g. when routing through an LLM gateway.",
        ),
        e(
            "AWS_BEARER_TOKEN_BEDROCK",
            CAT_AUTH,
            "Amazon Bedrock API key used for authentication.",
        ),
        // --- Feature Toggles ---
        e(
            "DISABLE_AUTOUPDATER",
            CAT_TOGGLES,
            "Set to 1 to disable automatic updates of the Claude Code CLI.",
        ),
        e(
            "DISABLE_TELEMETRY",
            CAT_TOGGLES,
            "Set to 1 to opt out of Statsig usage telemetry (does not affect OpenTelemetry).",
        ),
        e(
            "DISABLE_ERROR_REPORTING",
            CAT_TOGGLES,
            "Set to 1 to opt out of Sentry error reporting.",
        ),
        e(
            "DISABLE_COST_WARNINGS",
            CAT_TOGGLES,
            "Set to 1 to suppress cost warning messages.",
        ),
        e(
            "DISABLE_NON_ESSENTIAL_MODEL_CALLS",
            CAT_TOGGLES,
            "Set to 1 to disable model calls for non-critical paths such as conversation flavor text.",
        ),
        e(
            "DISABLE_PROMPT_CACHING",
            CAT_TOGGLES,
            "Set to 1 to disable prompt caching of request prefixes.",
        ),
        e(
            "DISABLE_BUG_COMMAND",
            CAT_TOGGLES,
            "Set to 1 to disable the /bug command.",
        ),
        e(
            "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
            CAT_TOGGLES,
            "Set to 1 to stop Claude Code from updating the terminal window title from conversation context.",
        ),
        e(
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            CAT_TOGGLES,
            "Set to 1 to disable all non-essential network traffic (autoupdater, telemetry, error reporting) with one flag.",
        ),
        // --- Telemetry & OpenTelemetry ---
        e(
            "CLAUDE_CODE_ENABLE_TELEMETRY",
            CAT_TELEMETRY,
            "Set to 1 to enable the OpenTelemetry metrics and events exporter.",
        ),
        e(
            "OTEL_METRICS_EXPORTER",
            CAT_TELEMETRY,
            "OpenTelemetry metrics exporter(s): otlp, prometheus, or console (comma-separated).",
        ),
        e(
            "OTEL_LOGS_EXPORTER",
            CAT_TELEMETRY,
            "OpenTelemetry logs/events exporter(s): otlp or console (comma-separated).",
        ),
        e(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            CAT_TELEMETRY,
            "Base URL of the OTLP receiver that metrics and logs are exported to.",
        ),
        e(
            "OTEL_EXPORTER_OTLP_PROTOCOL",
            CAT_TELEMETRY,
            "OTLP transport protocol: grpc, http/protobuf, or http/json.",
        ),
        e(
            "OTEL_LOG_USER_PROMPTS",
            CAT_TELEMETRY,
            "Set to 1 to include user prompt content in exported OpenTelemetry log events (off by default).",
        ),
        e(
            "OTEL_METRIC_EXPORT_INTERVAL",
            CAT_TELEMETRY,
            "Interval in milliseconds between OpenTelemetry metric exports.",
        ),
        // --- Behavior & Limits ---
        e(
            "BASH_DEFAULT_TIMEOUT_MS",
            CAT_BEHAVIOR,
            "Default timeout for long-running Bash commands (default 120000, i.e. 2 minutes).",
        ),
        e(
            "BASH_MAX_TIMEOUT_MS",
            CAT_BEHAVIOR,
            "Maximum timeout the model may set for long-running Bash commands (default 600000, i.e. 10 minutes).",
        ),
        e(
            "BASH_MAX_OUTPUT_LENGTH",
            CAT_BEHAVIOR,
            "Maximum characters in Bash output before it is saved to a file and only a preview is returned.",
        ),
        e(
            "MAX_THINKING_TOKENS",
            CAT_BEHAVIOR,
            "Fixed extended-thinking token budget for models that use a fixed budget; set 0 to force thinking off.",
        ),
        e(
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            CAT_BEHAVIOR,
            "Maximum number of output tokens requested for each model response.",
        ),
        e(
            "MAX_MCP_OUTPUT_TOKENS",
            CAT_BEHAVIOR,
            "Maximum number of tokens allowed in a single MCP tool response (default 25000).",
        ),
        e(
            "API_TIMEOUT_MS",
            CAT_BEHAVIOR,
            "Timeout for API requests in milliseconds (default 600000, i.e. 10 minutes).",
        ),
        e(
            "MCP_TIMEOUT",
            CAT_BEHAVIOR,
            "Timeout in milliseconds for MCP server startup and connection.",
        ),
        e(
            "MCP_TOOL_TIMEOUT",
            CAT_BEHAVIOR,
            "Timeout in milliseconds for a single MCP tool call to complete.",
        ),
        e(
            "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
            CAT_BEHAVIOR,
            "Interval in milliseconds at which credentials from apiKeyHelper are refreshed.",
        ),
        // --- Config, Paths & Proxy ---
        e(
            "CLAUDE_CONFIG_DIR",
            CAT_CONFIG,
            "Override the directory Claude Code reads and writes its configuration from (default ~/.claude).",
        ),
        e(
            "HTTP_PROXY",
            CAT_CONFIG,
            "Proxy server URL used for outbound HTTP requests.",
        ),
        e(
            "HTTPS_PROXY",
            CAT_CONFIG,
            "Proxy server URL used for outbound HTTPS requests.",
        ),
        e(
            "NO_PROXY",
            CAT_CONFIG,
            "Comma-separated list of hosts or domains that bypass the configured proxy.",
        ),
        e(
            "USE_BUILTIN_RIPGREP",
            CAT_CONFIG,
            "Set to 0 to use a system-installed ripgrep instead of the version bundled with Claude Code.",
        ),
        e(
            "CLAUDE_CODE_CERT_STORE",
            CAT_CONFIG,
            "Comma-separated CA certificate sources for TLS connections (bundled, system; default bundled,system).",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn env_catalog_nonempty_and_has_descriptions() {
        let cat = env_catalog();
        assert!(
            cat.len() >= 30,
            "catalog must have >= 30 entries; got {}",
            cat.len()
        );
        let mut seen: HashSet<&str> = HashSet::new();
        for def in &cat {
            assert!(!def.name.is_empty(), "an entry has an empty name");
            assert!(
                !def.description.is_empty(),
                "entry '{}' has an empty description",
                def.name
            );
            assert!(
                !def.category.is_empty(),
                "entry '{}' has an empty category",
                def.name
            );
            assert!(
                seen.insert(def.name.as_str()),
                "duplicate env var name '{}'",
                def.name
            );
        }
    }

    #[test]
    fn env_var_def_serializes_camel_case() {
        let d = EnvVarDef {
            name: "ANTHROPIC_API_KEY".into(),
            description: "API key sent as the X-Api-Key header.".into(),
            category: "Authentication & Provider".into(),
        };
        let s = serde_json::to_string(&d).unwrap();
        // The three fields all land on the wire.
        assert!(
            s.contains("\"name\":\"ANTHROPIC_API_KEY\""),
            "name present: {s}"
        );
        assert!(
            s.contains("\"description\":\"API key sent as the X-Api-Key header.\""),
            "description present: {s}"
        );
        assert!(
            s.contains("\"category\":\"Authentication & Provider\""),
            "category present: {s}"
        );
        // Round-trips value-for-value.
        let back: EnvVarDef = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    /// The catalog must actually cover the documented anchor set the Settings
    /// mode promises, spanning every category — guards against a future edit
    /// silently dropping a commonly-used variable.
    #[test]
    fn env_catalog_covers_required_anchor_vars() {
        let cat = env_catalog();
        let names: HashSet<&str> = cat.iter().map(|d| d.name.as_str()).collect();
        for required in [
            // Auth / provider
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "AWS_BEARER_TOKEN_BEDROCK",
            // Feature toggles
            "DISABLE_AUTOUPDATER",
            "DISABLE_TELEMETRY",
            "DISABLE_ERROR_REPORTING",
            "DISABLE_COST_WARNINGS",
            "DISABLE_NON_ESSENTIAL_MODEL_CALLS",
            "DISABLE_PROMPT_CACHING",
            "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
            // Telemetry / OTEL
            "CLAUDE_CODE_ENABLE_TELEMETRY",
            "OTEL_METRICS_EXPORTER",
            "OTEL_LOGS_EXPORTER",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_LOG_USER_PROMPTS",
            // Behavior / limits
            "BASH_DEFAULT_TIMEOUT_MS",
            "BASH_MAX_TIMEOUT_MS",
            "BASH_MAX_OUTPUT_LENGTH",
            "MAX_THINKING_TOKENS",
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            "MAX_MCP_OUTPUT_TOKENS",
            "API_TIMEOUT_MS",
            "MCP_TIMEOUT",
            "MCP_TOOL_TIMEOUT",
            "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
            // Config / paths / proxy
            "CLAUDE_CONFIG_DIR",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
        ] {
            assert!(
                names.contains(required),
                "catalog is missing the required env var '{required}'"
            );
        }
    }
}
