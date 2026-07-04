//! security/mod.rs — Public surface for the 4-layer security scanner.
//!
//! Modules:
//!   - `deobfuscate` — Layer 1: 8 deobfuscation techniques.
//!   - `rules`       — Layer 2: ~60 regex rules (CCO parity).
//!   - `baseline`    — Layer 3: SHA-256 baseline under `~/.ward/security/`.
//!   - `judge`       — Layer 4: optional `claude -p` LLM judge.
//!   - `scan`        — Orchestration + dedup detection.

pub mod baseline;
pub mod deobfuscate;
pub mod judge;
pub mod rules;
pub mod scan;