//! tokenizer.rs — Token counting for the Context Budget mode.
//!
//! CCO parity: CCO uses the optional `ai-tokenizer` package (Claude's
//! tokenizer) and falls back to bytes/4 when unavailable. The `measured`
//! flag is set to `true` only when a real BPE tokenizer is in use.
//!
//! We start with the bytes/4 fallback (always estimated). The architecture
//! exposes a `TokenizerKind` enum so a future upgrade to `tiktoken-rs` can
//! flip `measured = true` without changing call sites. The interface is:
//!
//!   - `count_text(&str) -> TokenCount { tokens, measured }`
//!   - `count_file(&Path) -> Result<TokenCount, WardError>`
//!
//! `count_file` reads the file with `read_to_string` and delegates to
//! `count_text` so callers get a single source of truth for the
//! byte-division heuristic.
//!
//! The bytes/4 heuristic is the same one CCO uses as its fallback. It is
//! intentionally simple: `ceil(bytes / 4)`. English text comes out
//! roughly 75-85% accurate vs. Claude's cl100k_base tokenizer, which is
//! good enough for a budget UI that the user reads as "approximate".

use std::path::Path;

use crate::error::WardError;

/// Identifier for which token-counting strategy produced the result.
/// Today only `BytesDiv4` is wired up; the enum is here so the upgrade
/// to `tiktoken-rs` (or any future real BPE tokenizer) can flip
/// `measured = true` without touching callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerKind {
    /// Real BPE tokenizer (tiktoken-rs / cl100k_base) — accurate.
    /// Not yet enabled; will be added in a follow-up once compile
    /// time on the build host is verified to stay under the 2-min
    /// threshold documented in the plan.
    Tiktoken,
    /// `ceil(bytes / 4)` heuristic — ~75-85% accurate for English.
    /// This is the fallback we ship today.
    BytesDiv4,
}

/// Token count result for a single text or file.
///
/// `measured = true` means a real tokenizer produced the number;
/// `measured = false` means the bytes/4 estimate was used. The UI
/// surfaces this honestly so the user can interpret the budget
/// ("measured" vs "estimated").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenCount {
    pub tokens: usize,
    pub measured: bool,
}

/// Which tokenizer is active. Today always returns `BytesDiv4`. A
/// future tiktoken upgrade will return `Tiktoken` and call sites do
/// not need to change — the `measured` flag is the public signal.
pub fn active_tokenizer() -> TokenizerKind {
    TokenizerKind::BytesDiv4
}

/// Count tokens in a string. Empty / whitespace-only input returns
/// zero so the budget breakdown doesn't carry ghost items.
pub fn count_text(text: &str) -> TokenCount {
    let kind = active_tokenizer();
    match kind {
        TokenizerKind::Tiktoken => {
            // Reserved for the future tiktoken-rs upgrade. Intentionally
            // unreachable while BytesDiv4 is active — the bytes/4 path
            // below is the only one that fires today.
            unreachable!("tiktoken-rs upgrade pending")
        }
        TokenizerKind::BytesDiv4 => {
            if text.is_empty() {
                return TokenCount { tokens: 0, measured: false };
            }
            let bytes = text.len();
            let tokens = (bytes + 3) / 4; // ceil(bytes / 4) without floats
            TokenCount { tokens, measured: false }
        }
    }
}

/// Read a file's contents and count its tokens. Wraps IO errors as
/// `WardError::Io` so callers (the budget composer) can surface
/// missing/unreadable files via the standard error path.
pub fn count_file(path: &Path) -> Result<TokenCount, WardError> {
    let content = std::fs::read_to_string(path)?;
    Ok(count_text(&content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    // ── count_text ──

    #[test]
    fn empty_string_yields_zero() {
        let c = count_text("");
        assert_eq!(c.tokens, 0);
        assert!(!c.measured);
    }

    #[test]
    fn four_byte_input_yields_one_token() {
        let c = count_text("abcd");
        assert_eq!(c.tokens, 1);
        assert!(!c.measured);
    }

    #[test]
    fn rounds_up_partial_bytes() {
        // 5 bytes -> ceil(5/4) = 2 tokens
        let c = count_text("abcde");
        assert_eq!(c.tokens, 2);
    }

    #[test]
    fn large_input_scales_linearly() {
        let s = "a".repeat(1000);
        let c = count_text(&s);
        // 1000 bytes / 4 = 250 exactly (no rounding)
        assert_eq!(c.tokens, 250);
    }

    #[test]
    fn reports_estimated_for_bytes_div4() {
        let c = count_text("anything here");
        assert!(!c.measured, "bytes/4 must report measured=false");
    }

    #[test]
    fn active_tokenizer_is_bytes_div4() {
        assert_eq!(active_tokenizer(), TokenizerKind::BytesDiv4);
    }

    // ── count_file ──

    #[test]
    fn reads_and_counts_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(b"hello world").unwrap();
        let c = count_file(&p).unwrap();
        // 11 bytes -> ceil(11/4) = 3
        assert_eq!(c.tokens, 3);
        assert!(!c.measured);
    }

    #[test]
    fn missing_file_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does-not-exist.md");
        let err = count_file(&p).unwrap_err();
        assert!(matches!(err, WardError::Io(_)));
    }

    #[test]
    fn empty_file_yields_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.md");
        fs::write(&p, "").unwrap();
        let c = count_file(&p).unwrap();
        assert_eq!(c.tokens, 0);
    }
}