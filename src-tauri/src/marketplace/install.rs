//! Plan 21 Task 3 — build a version-pinned, secret-safe MCP config and
//! fan the install out to the shared `upsert_mcp_entry` engine.
//!
//! `build_mcp_config` is pure and fully unit-tested (version-pin enforcement,
//! secret omission, stdio/remote shapes). `install` dispatches per target via
//! `ops_for(&harness)?.upsert_mcp_entry(...)`, collecting one `InstallResult`
//! per target so a single failure never aborts the batch — and it reuses the
//! EXACT same dispatch the Organizer's Save uses (no second MCP writer).

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};

use super::{BuiltConfig, EnvVar, InstallResult, InstallTarget, MarketEntry, Package, Remote};
use crate::error::WardError;
use crate::harness::{Ctx, HarnessOps};
use crate::model::{RestoreInfo, Scope};

/// The exact message the version-pin guard raises (spec §9.5).
const UNPINNED: &str = "refusing to install an unpinned version";

// ── Build (pure) ─────────────────────────────────────────────────────────

/// Build the exact server object that will land on disk for `entry`'s
/// `package_index` package (or, when the entry is remote-only, its
/// `package_index` remote). Pure and fully unit-tested.
///
/// Security (spec §9.5, enforced here):
///   * **Version pin** — rejects an empty or `latest` version with
///     [`WardError::Registry`]; npm → `npx -y <id>@<ver>`, pypi →
///     `uvx <id>==<ver>`. OCI is surfaced, never fabricated into a
///     `docker run`.
///   * **Secret-safe** — a secret env var / header is NEVER written to the
///     config (it would clobber the real value the user sets in their
///     environment); it is only surfaced in `BuiltConfig.env` metadata.
///     Non-secret vars with a provided value are written.
pub fn build_mcp_config(
    entry: &MarketEntry,
    package_index: usize,
    env_values: &HashMap<String, String>,
) -> Result<BuiltConfig, WardError> {
    if !entry.packages.is_empty() {
        let pkg = entry.packages.get(package_index).ok_or_else(|| {
            WardError::Registry(format!(
                "package index {package_index} out of range for '{}'",
                entry.name
            ))
        })?;
        build_from_package(entry, pkg, env_values)
    } else if !entry.remotes.is_empty() {
        // Entry with no packages → install its remote at the same index.
        let remote = entry.remotes.get(package_index).ok_or_else(|| {
            WardError::Registry(format!(
                "remote index {package_index} out of range for '{}'",
                entry.name
            ))
        })?;
        build_from_remote(entry, remote, env_values)
    } else {
        Err(WardError::Registry(format!(
            "entry '{}' has no packages or remotes to install",
            entry.name
        )))
    }
}

/// Derive a clean local MCP server key from the registry id. The registry
/// name is a namespaced id (`io.github.acme/notes`); we key the server under
/// the last path segment (`notes`), the way MCP servers are usually named.
fn derive_local_name(registry_name: &str) -> String {
    let seg = registry_name.rsplit('/').next().unwrap_or("").trim();
    if seg.is_empty() {
        registry_name.trim().to_string()
    } else {
        seg.to_string()
    }
}

/// Reject an empty or `latest` version — Ward never installs an unpinned
/// package (spec §9.5).
fn reject_unpinned(version: &str) -> Result<(), WardError> {
    let v = version.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("latest") {
        return Err(WardError::Registry(UNPINNED.into()));
    }
    Ok(())
}

/// Build the `env`/`headers` object (secret-safe) alongside the full var
/// metadata. Secret vars are omitted from the written object entirely — never
/// a token to disk, and never a clobbering empty string that would shadow the
/// value the user sets in their shell. Non-secret vars with a non-empty
/// provided value are written. Every declared var is echoed in the returned
/// metadata so the UI can render it (and mark secrets read-only).
fn build_secret_safe_object(
    vars: &[EnvVar],
    env_values: &HashMap<String, String>,
) -> (Map<String, Value>, Vec<EnvVar>) {
    let mut obj = Map::new();
    for v in vars {
        if v.is_secret {
            continue; // surfaced via metadata + the user's environment only
        }
        if let Some(val) = env_values.get(&v.name) {
            if !val.is_empty() {
                obj.insert(v.name.clone(), Value::String(val.clone()));
            }
        }
    }
    (obj, vars.to_vec())
}

fn build_from_package(
    entry: &MarketEntry,
    pkg: &Package,
    env_values: &HashMap<String, String>,
) -> Result<BuiltConfig, WardError> {
    let version = pkg.version.trim();
    reject_unpinned(version)?;

    let (command, args): (String, Vec<String>) = match pkg.registry_type.as_str() {
        "npm" => (
            "npx".into(),
            vec!["-y".into(), format!("{}@{}", pkg.identifier, version)],
        ),
        "pypi" => ("uvx".into(), vec![format!("{}=={}", pkg.identifier, version)]),
        "oci" => {
            // Surface, but do NOT fabricate a `docker run` — Ward never
            // silently launches a container (spec §12).
            return Err(WardError::Registry(format!(
                "'{}' ships an OCI/container image ({}:{}); install it with your container runtime — Ward will not run docker for you",
                entry.name, pkg.identifier, version
            )));
        }
        other => {
            return Err(WardError::Registry(format!(
                "unsupported package registry type '{other}' for '{}'",
                entry.name
            )));
        }
    };

    let (env_obj, env_meta) = build_secret_safe_object(&pkg.env, env_values);

    // Flattened preview BEFORE we move command/args into the config.
    let mut command_preview = Vec::with_capacity(1 + args.len());
    command_preview.push(command.clone());
    command_preview.extend(args.iter().cloned());

    let mut config = Map::new();
    config.insert("command".into(), Value::String(command));
    config.insert(
        "args".into(),
        Value::Array(args.into_iter().map(Value::String).collect()),
    );
    if !env_obj.is_empty() {
        config.insert("env".into(), Value::Object(env_obj));
    }

    Ok(BuiltConfig {
        name: derive_local_name(&entry.name),
        config: Value::Object(config),
        command_preview,
        env: env_meta,
    })
}

fn build_from_remote(
    entry: &MarketEntry,
    remote: &Remote,
    env_values: &HashMap<String, String>,
) -> Result<BuiltConfig, WardError> {
    let url = remote.url.trim();
    if url.is_empty() {
        return Err(WardError::Registry(format!(
            "remote for '{}' has no url",
            entry.name
        )));
    }
    let (headers_obj, headers_meta) = build_secret_safe_object(&remote.headers, env_values);

    let mut config = Map::new();
    config.insert("url".into(), Value::String(url.to_string()));
    // Map the registry transport onto the `type` Claude Code understands
    // (`http` / `sse`); the registry's `streamable-http` becomes `http`.
    config.insert("type".into(), Value::String(map_remote_type(&remote.transport)));
    if !headers_obj.is_empty() {
        config.insert("headers".into(), Value::Object(headers_obj));
    }

    Ok(BuiltConfig {
        name: derive_local_name(&entry.name),
        config: Value::Object(config),
        command_preview: vec![url.to_string()],
        env: headers_meta,
    })
}

/// Registry remote transport → Claude Code config `type`. Claude accepts
/// `http` and `sse`; anything streamable/http maps to `http`.
fn map_remote_type(transport: &str) -> String {
    match transport.to_ascii_lowercase().as_str() {
        "sse" => "sse".into(),
        _ => "http".into(),
    }
}

// ── Install fan-out ──────────────────────────────────────────────────────

/// Install `entry`'s `package_index` package into every target, reusing the
/// shared `upsert_mcp_entry` engine. One `InstallResult` per target; a single
/// target's failure never aborts the batch (each success is independently
/// undoable via its `restore`). Network-free — the registry was already
/// fetched; this only writes local config files.
pub fn install(
    entry: &MarketEntry,
    package_index: usize,
    targets: &[InstallTarget],
    env_values: &HashMap<String, String>,
) -> Vec<InstallResult> {
    // Skills fan out through `skill_upsert` (fetch the SKILL.md, write it into
    // each target); MCP servers fan out through `upsert_mcp_entry`. Both reuse
    // the SAME write engines the Organizer uses — no second writer.
    if entry.kind == "skill" {
        return install_skill(entry, targets);
    }
    install_with(entry, package_index, targets, env_values, |harness| {
        let ops = crate::commands::ops_for(harness)?;
        let (ctx, scopes) = crate::commands::harness_ctx(harness)?;
        Ok((ops, ctx, scopes))
    })
}

/// Install a skill entry into every target. Fetches the `SKILL.md` **once**
/// (the same bytes land in every target) then writes it via the shared
/// create-only `skill_upsert`. Network reaches out here (unlike the MCP path,
/// where the registry was already fetched) — it is still user-triggered.
fn install_skill(entry: &MarketEntry, targets: &[InstallTarget]) -> Vec<InstallResult> {
    install_skill_with(
        entry,
        targets,
        crate::marketplace::skills::fetch_skill_md,
        |harness| {
            let (ctx, scopes) = crate::commands::harness_ctx(harness)?;
            Ok((ctx.home, scopes))
        },
    )
}

/// The skill install loop, with the network fetch and per-harness resolution
/// injected so the fan-out (fetch once → write each → never abort) is
/// unit-testable against a temp home without touching the network or `~/.claude`.
fn install_skill_with<Fetch, Resolve>(
    entry: &MarketEntry,
    targets: &[InstallTarget],
    fetch: Fetch,
    resolve: Resolve,
) -> Vec<InstallResult>
where
    Fetch: Fn(&str) -> Result<String, WardError>,
    Resolve: Fn(&str) -> Result<(&'static Path, Vec<Scope>), WardError>,
{
    // Fetch the SKILL.md exactly once — identical content for every target.
    // Held as a `Result<String, String>` so each target can re-surface a shared
    // fetch failure without needing `WardError: Clone`.
    let content: Result<String, String> = match entry.skill_path.as_deref().filter(|s| !s.is_empty())
    {
        Some(url) => fetch(url).map_err(|e| e.to_string()),
        None => Err(format!("skill '{}' has no source URL to fetch", entry.name)),
    };

    targets
        .iter()
        .map(|target| {
            let outcome: Result<RestoreInfo, WardError> = (|| {
                let body = content.as_ref().map_err(|e| WardError::Registry(e.clone()))?;
                let (home, scopes) = resolve(&target.harness)?;
                crate::harness::adapters::claude_ops::skill_upsert(
                    home,
                    &target.harness,
                    &target.scope_id,
                    &entry.name,
                    body,
                    &scopes,
                )
            })();
            match outcome {
                Ok(restore) => InstallResult {
                    target: target.clone(),
                    ok: true,
                    error: None,
                    restore: Some(restore),
                },
                Err(e) => InstallResult {
                    target: target.clone(),
                    ok: false,
                    error: Some(e.to_string()),
                    restore: None,
                },
            }
        })
        .collect()
}

/// The install loop, with the per-harness resolution injected so the fan-out
/// (build → dispatch → collect, never abort) is unit-testable against a temp
/// home without touching the real `~/.claude`.
fn install_with<F>(
    entry: &MarketEntry,
    package_index: usize,
    targets: &[InstallTarget],
    env_values: &HashMap<String, String>,
    resolve: F,
) -> Vec<InstallResult>
where
    F: Fn(&str) -> Result<(&'static dyn HarnessOps, Ctx<'static>, Vec<Scope>), WardError>,
{
    targets
        .iter()
        .map(|target| {
            let outcome = install_one(entry, package_index, target, env_values, &resolve);
            match outcome {
                Ok(restore) => InstallResult {
                    target: target.clone(),
                    ok: true,
                    error: None,
                    restore: Some(restore),
                },
                Err(e) => InstallResult {
                    target: target.clone(),
                    ok: false,
                    error: Some(e.to_string()),
                    restore: None,
                },
            }
        })
        .collect()
}

fn install_one<F>(
    entry: &MarketEntry,
    package_index: usize,
    target: &InstallTarget,
    env_values: &HashMap<String, String>,
    resolve: &F,
) -> Result<RestoreInfo, WardError>
where
    F: Fn(&str) -> Result<(&'static dyn HarnessOps, Ctx<'static>, Vec<Scope>), WardError>,
{
    let built = build_mcp_config(entry, package_index, env_values)?;
    let (ops, ctx, scopes) = resolve(&target.harness)?;
    // target_path = None → Rust resolves the scope's write target (a new
    // server), exactly like the Organizer's "+ Add MCP".
    ops.upsert_mcp_entry(&ctx, &target.scope_id, &built.name, &built.config, None, &scopes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::harness::adapters::claude::ClaudeAdapter;
    use crate::harness::adapters::claude_ops::ClaudeOps;
    use crate::harness::framework;

    fn npm_entry() -> MarketEntry {
        MarketEntry {
            kind: "mcp".into(),
            name: "io.github.acme/notes".into(),
            display_name: "Acme Notes".into(),
            description: "notes".into(),
            source: "registry".into(),
            version: Some("2.1.0".into()),
            verified: true,
            packages: vec![Package {
                registry_type: "npm".into(),
                identifier: "@acme/notes-mcp".into(),
                version: "2.1.0".into(),
                transport: "stdio".into(),
                env: vec![
                    EnvVar { name: "NOTES_API_KEY".into(), is_required: true, is_secret: true },
                    EnvVar { name: "NOTES_REGION".into(), is_required: false, is_secret: false },
                ],
                runtime_hint: None,
            }],
            remotes: vec![],
            repo_url: None,
            skill_path: None,
        }
    }

    fn pypi_entry() -> MarketEntry {
        MarketEntry {
            kind: "mcp".into(),
            name: "io.github.acme/pytools".into(),
            display_name: "pytools".into(),
            description: "py".into(),
            source: "registry".into(),
            version: Some("0.4.2".into()),
            verified: true,
            packages: vec![Package {
                registry_type: "pypi".into(),
                identifier: "acme-pytools".into(),
                version: "0.4.2".into(),
                transport: "stdio".into(),
                env: vec![],
                runtime_hint: Some("uvx".into()),
            }],
            remotes: vec![],
            repo_url: None,
            skill_path: None,
        }
    }

    fn remote_entry() -> MarketEntry {
        MarketEntry {
            kind: "mcp".into(),
            name: "com.acme/hosted".into(),
            display_name: "Acme Hosted".into(),
            description: "hosted".into(),
            source: "registry".into(),
            version: Some("3.0.0".into()),
            verified: true,
            packages: vec![],
            remotes: vec![Remote {
                transport: "streamable-http".into(),
                url: "https://mcp.acme.example/v1".into(),
                headers: vec![
                    EnvVar { name: "X-Acme-Token".into(), is_required: true, is_secret: true },
                    EnvVar { name: "X-Acme-Region".into(), is_required: false, is_secret: false },
                ],
            }],
            repo_url: None,
            skill_path: None,
        }
    }

    #[test]
    fn npm_pins_version_and_uses_npx() {
        let built = build_mcp_config(&npm_entry(), 0, &HashMap::new()).unwrap();
        assert_eq!(built.name, "notes");
        assert_eq!(built.config["command"], "npx");
        assert_eq!(built.config["args"], serde_json::json!(["-y", "@acme/notes-mcp@2.1.0"]));
        assert_eq!(built.command_preview, vec!["npx", "-y", "@acme/notes-mcp@2.1.0"]);
        // Never @latest.
        assert!(!built.command_preview.iter().any(|a| a.contains("@latest")));
    }

    #[test]
    fn pypi_pins_version_with_uvx() {
        let built = build_mcp_config(&pypi_entry(), 0, &HashMap::new()).unwrap();
        assert_eq!(built.name, "pytools");
        assert_eq!(built.config["command"], "uvx");
        assert_eq!(built.config["args"], serde_json::json!(["acme-pytools==0.4.2"]));
        assert_eq!(built.command_preview, vec!["uvx", "acme-pytools==0.4.2"]);
    }

    #[test]
    fn rejects_latest_and_empty_versions() {
        let mut e = npm_entry();
        e.packages[0].version = "latest".into();
        let err = build_mcp_config(&e, 0, &HashMap::new()).unwrap_err();
        assert!(matches!(&err, WardError::Registry(m) if m == UNPINNED), "got {err:?}");

        e.packages[0].version = "  ".into();
        let err = build_mcp_config(&e, 0, &HashMap::new()).unwrap_err();
        assert!(matches!(&err, WardError::Registry(m) if m == UNPINNED), "got {err:?}");

        // Case-insensitive on "LATEST".
        e.packages[0].version = "LATEST".into();
        assert!(build_mcp_config(&e, 0, &HashMap::new()).is_err());
    }

    #[test]
    fn omits_secret_env_and_writes_provided_nonsecret() {
        let mut env_values = HashMap::new();
        env_values.insert("NOTES_REGION".to_string(), "us-east-1".to_string());
        // A secret value MUST be ignored even if (mistakenly) provided.
        env_values.insert("NOTES_API_KEY".to_string(), "sk-should-be-dropped".to_string());

        let built = build_mcp_config(&npm_entry(), 0, &env_values).unwrap();
        let env = built.config["env"].as_object().unwrap();
        assert_eq!(env.get("NOTES_REGION").and_then(|v| v.as_str()), Some("us-east-1"));
        assert!(!env.contains_key("NOTES_API_KEY"), "secret must never be written to disk");
        // The full config JSON must not carry the secret value anywhere.
        assert!(!serde_json::to_string(&built.config).unwrap().contains("sk-should-be-dropped"));
        // Metadata still lists BOTH vars so the UI can render them.
        assert_eq!(built.env.len(), 2);
        assert!(built.env.iter().any(|v| v.name == "NOTES_API_KEY" && v.is_secret));
        assert!(built.env.iter().any(|v| v.name == "NOTES_REGION" && !v.is_secret));
    }

    #[test]
    fn unfilled_nonsecret_env_is_omitted_no_env_key() {
        // Nothing provided → no env object at all (don't clobber the shell).
        let built = build_mcp_config(&npm_entry(), 0, &HashMap::new()).unwrap();
        assert!(built.config.get("env").is_none(), "no env values → no env key");
    }

    #[test]
    fn remote_shape_maps_type_and_omits_secret_headers() {
        let mut env_values = HashMap::new();
        env_values.insert("X-Acme-Region".to_string(), "eu".to_string());
        let built = build_mcp_config(&remote_entry(), 0, &env_values).unwrap();
        assert_eq!(built.name, "hosted");
        assert_eq!(built.config["url"], "https://mcp.acme.example/v1");
        assert_eq!(built.config["type"], "http"); // streamable-http → http
        let headers = built.config["headers"].as_object().unwrap();
        assert_eq!(headers.get("X-Acme-Region").and_then(|v| v.as_str()), Some("eu"));
        assert!(!headers.contains_key("X-Acme-Token"), "secret header must never be written");
        assert_eq!(built.command_preview, vec!["https://mcp.acme.example/v1"]);
        assert_eq!(built.config.get("command"), None, "remote config has no command");
    }

    #[test]
    fn oci_is_surfaced_not_fabricated() {
        let mut e = npm_entry();
        e.packages[0].registry_type = "oci".into();
        e.packages[0].identifier = "ghcr.io/acme/notes".into();
        let err = build_mcp_config(&e, 0, &HashMap::new()).unwrap_err();
        match err {
            WardError::Registry(m) => {
                assert!(m.contains("OCI") || m.contains("docker"), "message should surface OCI: {m}");
                assert!(!m.contains("docker run"), "must not fabricate a docker run command");
            }
            other => panic!("expected Registry error, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_and_empty_entry_error() {
        let err = build_mcp_config(&npm_entry(), 9, &HashMap::new()).unwrap_err();
        assert!(matches!(err, WardError::Registry(_)));

        let empty = MarketEntry {
            kind: "mcp".into(), name: "x/empty".into(), display_name: "x".into(),
            description: String::new(), source: "registry".into(), version: None,
            verified: true, packages: vec![], remotes: vec![], repo_url: None, skill_path: None,
        };
        assert!(build_mcp_config(&empty, 0, &HashMap::new()).is_err());
    }

    /// Integration: the built config actually lands via the ClaudeOps upsert
    /// path against a temp home, and a single bad target neither aborts the
    /// batch nor writes anything. Mirrors the commands.rs ops-path test style.
    #[test]
    fn install_writes_via_claude_ops_and_collects_partial_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let scopes = vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global".into(),
            root: home.join(".claude").display().to_string(),
        }];
        // Leak the home so the injected resolver can hand out Ctx<'static>.
        let home_static: &'static Path = Box::leak(home.to_path_buf().into_boxed_path());
        let scopes_for_closure = scopes.clone();

        let entry = npm_entry();
        let targets = vec![
            InstallTarget { harness: "claude".into(), scope_id: "global".into() },
            InstallTarget { harness: "nope".into(), scope_id: "global".into() },
        ];
        let results = install_with(&entry, 0, &targets, &HashMap::new(), move |h| match h {
            "claude" => Ok((
                &ClaudeOps as &'static dyn HarnessOps,
                Ctx { home: home_static, cwd: None },
                scopes_for_closure.clone(),
            )),
            other => Err(WardError::HarnessUnavailable(other.into())),
        });

        assert_eq!(results.len(), 2, "every target attempted; no early abort");
        assert!(results[0].ok, "claude target should succeed: {:?}", results[0].error);
        assert_eq!(results[0].restore.as_ref().unwrap().kind, "mcp-upsert");
        assert!(!results[1].ok, "bad harness must fail");
        assert!(results[1].error.as_ref().unwrap().to_lowercase().contains("harness"));

        // Scan-visibility: the write target is a scanned file, so the server
        // shows up on a fresh scan.
        let ctx = Ctx { home, cwd: None };
        let scan = framework::run_scan(&ClaudeAdapter, &ctx).unwrap();
        assert!(
            scan.items.iter().any(|i| i.category == "mcp" && i.name == "notes"),
            "installed MCP server should appear in the scan"
        );
    }

    // ── Skill install fan-out (Plan 22) ──────────────────────────────────

    fn skill_entry() -> MarketEntry {
        MarketEntry {
            kind: "skill".into(),
            name: "brainstorming".into(),
            display_name: "brainstorming".into(),
            description: "Explore intent before building.".into(),
            source: "marketplace".into(),
            version: None,
            verified: true,
            packages: vec![],
            remotes: vec![],
            repo_url: Some("https://raw.githubusercontent.com/acme/agent-skills/main".into()),
            skill_path: Some(
                "https://raw.githubusercontent.com/acme/agent-skills/main/skills/brainstorming/SKILL.md".into(),
            ),
        }
    }

    /// The SKILL.md is fetched once (shared content), written into each target
    /// via `skill_upsert`, and a single bad target neither aborts the batch nor
    /// blocks the good one. Mirrors the MCP install test.
    #[test]
    fn install_skill_writes_skill_md_and_collects_partial_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let scopes = vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global".into(),
            root: home.join(".claude").display().to_string(),
        }];
        let home_static: &'static Path = Box::leak(home.to_path_buf().into_boxed_path());
        let scopes_for_closure = scopes.clone();

        let entry = skill_entry();
        let body = "---\nname: brainstorming\ndescription: d\n---\n\n# Brainstorming\n";
        let targets = vec![
            InstallTarget { harness: "claude".into(), scope_id: "global".into() },
            InstallTarget { harness: "nope".into(), scope_id: "global".into() },
        ];
        let results = install_skill_with(
            &entry,
            &targets,
            |_url| Ok(body.to_string()),
            move |h| match h {
                "claude" => Ok((home_static, scopes_for_closure.clone())),
                other => Err(WardError::HarnessUnavailable(other.into())),
            },
        );

        assert_eq!(results.len(), 2, "every target attempted; no early abort");
        assert!(results[0].ok, "claude skill install should succeed: {:?}", results[0].error);
        assert_eq!(results[0].restore.as_ref().unwrap().kind, "skill-create");
        assert!(!results[1].ok, "bad harness must fail");

        // The SKILL.md landed on disk with the EXACT fetched bytes.
        let written =
            std::fs::read_to_string(home.join(".claude/skills/brainstorming/SKILL.md")).unwrap();
        assert_eq!(written, body);

        // Scan-visibility: the new skill shows up on a fresh scan.
        let ctx = Ctx { home, cwd: None };
        let scan = framework::run_scan(&ClaudeAdapter, &ctx).unwrap();
        assert!(
            scan.items.iter().any(|i| i.category == "skill" && i.name == "brainstorming"),
            "installed skill should appear in the scan"
        );
    }

    /// A shared-fetch failure fails every target identically and writes nothing.
    #[test]
    fn install_skill_fetch_failure_fails_all_targets_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let scopes = vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global".into(),
            root: home.join(".claude").display().to_string(),
        }];
        let home_static: &'static Path = Box::leak(home.to_path_buf().into_boxed_path());
        let scopes_for_closure = scopes.clone();

        let entry = skill_entry();
        let targets = vec![
            InstallTarget { harness: "claude".into(), scope_id: "global".into() },
            InstallTarget { harness: "claude".into(), scope_id: "global".into() },
        ];
        let results = install_skill_with(
            &entry,
            &targets,
            |_url| Err(WardError::Registry("boom".into())),
            move |_h| Ok((home_static, scopes_for_closure.clone())),
        );
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| !r.ok));
        assert!(results[0].error.as_ref().unwrap().contains("boom"));
        assert!(!home.join(".claude/skills").exists(), "nothing written when the fetch fails");
    }

    /// A skill entry with no `skill_path` fails cleanly (still one result/target).
    #[test]
    fn install_skill_without_source_url_errors_per_target() {
        let mut entry = skill_entry();
        entry.skill_path = None;
        let targets = vec![InstallTarget { harness: "claude".into(), scope_id: "global".into() }];
        let results = install_skill_with(
            &entry,
            &targets,
            |_url| Ok("unused".to_string()),
            |_h| panic!("resolve must not be reached without content"),
        );
        assert_eq!(results.len(), 1);
        assert!(!results[0].ok);
        assert!(results[0].error.as_ref().unwrap().contains("no source URL"));
    }
}
