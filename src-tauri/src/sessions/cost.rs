//! Per-model cost aggregation for a parsed `Conversation`.
//!
//! The pricing table below is a **rough** port of publicly published
//! Anthropic pricing for the popular Claude models (Opus 4.1/4.2,
//! Sonnet 4.5/4.6, Haiku 4.5). Cache reads are billed at 0.1× input
//! and cache writes at 1.25× input — these multipliers are stable
//! across the Claude 3.x/4.x line.
//!
//! The UI surfaces every value as "estimated USD" — the parser is
//! honest that the per-token cost is a moving target and the user
//! should not treat the dollar total as authoritative.

use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::sessions::parse::{Conversation, SessionRecord};

/// All values are USD per **million** tokens.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ModelPrice {
    pub(crate) input_per_mtok: f64,
    pub(crate) output_per_mtok: f64,
    pub(crate) cache_read_multiplier: f64,
    pub(crate) cache_write_multiplier: f64,
}

/// Return the pricing row for `model`. Falls back to a conservative
/// Sonnet-4 estimate for unknown models so the UI never produces an
/// "unknown model" error — it just labels the breakdown "estimated".
pub(crate) fn price_for(model: &str) -> ModelPrice {
    // Match the longest-known family first so `claude-opus-4-1-20250514`
    // resolves to the Opus 4.1 row, not a generic Claude 4 entry.
    const OPUS_4_1: ModelPrice = ModelPrice {
        input_per_mtok: 15.0,
        output_per_mtok: 75.0,
        cache_read_multiplier: 0.1,
        cache_write_multiplier: 1.25,
    };
    const OPUS_4_2: ModelPrice = ModelPrice {
        input_per_mtok: 15.0,
        output_per_mtok: 75.0,
        cache_read_multiplier: 0.1,
        cache_write_multiplier: 1.25,
    };
    const SONNET_4_5: ModelPrice = ModelPrice {
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
        cache_read_multiplier: 0.1,
        cache_write_multiplier: 1.25,
    };
    const SONNET_4_6: ModelPrice = ModelPrice {
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
        cache_read_multiplier: 0.1,
        cache_write_multiplier: 1.25,
    };
    const HAIKU_4_5: ModelPrice = ModelPrice {
        input_per_mtok: 1.0,
        output_per_mtok: 5.0,
        cache_read_multiplier: 0.1,
        cache_write_multiplier: 1.25,
    };

    if model.contains("opus-4-2") {
        OPUS_4_2
    } else if model.contains("opus-4-1") || model.contains("opus-4") {
        OPUS_4_1
    } else if model.contains("sonnet-4-6") {
        SONNET_4_6
    } else if model.contains("sonnet-4-5") || model.contains("sonnet-4") {
        SONNET_4_5
    } else if model.contains("haiku-4-5") || model.contains("haiku-4") {
        HAIKU_4_5
    } else {
        // Conservative Sonnet-4 fallback.
        SONNET_4_5
    }
}

/// Per-model row in the cost breakdown.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
}

/// Aggregated cost result for a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostBreakdown {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub per_model: Vec<ModelCost>,
    pub estimated_cost_usd: f64,
    /// Number of assistant records whose model was unknown and fell
    /// through to the fallback price. Surfaced in the UI as a soft
    /// "estimated" tag so users know the breakdown is approximate.
    pub estimated_records: u64,
}

impl CostBreakdown {
    fn empty() -> Self {
        Self {
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read: 0,
            total_cache_write: 0,
            per_model: Vec::new(),
            estimated_cost_usd: 0.0,
            estimated_records: 0,
        }
    }
}

/// Walk every Assistant record and aggregate Usage by model string.
/// Returns a `CostBreakdown` with per-model rows and a grand total.
pub fn compute(conv: &Conversation) -> Result<CostBreakdown, WardError> {
    // Intermediate map: model name → running totals.
    use std::collections::BTreeMap;
    #[derive(Default, Clone)]
    struct Agg {
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        cost: f64,
        estimated: u64,
    }
    let mut map: BTreeMap<String, Agg> = BTreeMap::new();

    for rec in &conv.records {
        if let SessionRecord::Assistant { model, usage: Some(u), .. } = rec {
            let model_name = model.clone().unwrap_or_else(|| "unknown".to_string());
            let row = map.entry(model_name.clone()).or_default();
            row.input += u.input_tokens;
            row.output += u.output_tokens;
            if let Some(cr) = u.cache_read { row.cache_read += cr; }
            if let Some(cw) = u.cache_write { row.cache_write += cw; }

            let price = price_for(&model_name);
            let cost = cost_for(u, price);
            row.cost += cost;
            // Fallback price model is the Sonnet 4.5 estimate; we mark
            // the row "estimated" only when the model name did not
            // match any known family.
            if !is_known_model(&model_name) {
                row.estimated += 1;
            }
        }
    }

    let mut total_input = 0;
    let mut total_output = 0;
    let mut total_cache_read = 0;
    let mut total_cache_write = 0;
    let mut estimated_records = 0u64;
    let mut per_model: Vec<ModelCost> = Vec::with_capacity(map.len());
    let mut total_cost = 0.0_f64;

    for (model, agg) in map {
        total_input += agg.input;
        total_output += agg.output;
        total_cache_read += agg.cache_read;
        total_cache_write += agg.cache_write;
        estimated_records += agg.estimated;
        total_cost += agg.cost;
        per_model.push(ModelCost {
            model,
            input_tokens: agg.input,
            output_tokens: agg.output,
            cache_read: agg.cache_read,
            cache_write: agg.cache_write,
            cost_usd: round_usd(agg.cost),
        });
    }
    // Sort per-model rows by descending cost so the most expensive
    // model shows up first in the UI.
    per_model.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));

    Ok(CostBreakdown {
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read,
        total_cache_write,
        per_model,
        estimated_cost_usd: round_usd(total_cost),
        estimated_records,
    })
}

/// Returns true if `model` matches a known Claude family.
fn is_known_model(model: &str) -> bool {
    model.contains("opus") || model.contains("sonnet") || model.contains("haiku")
}

pub(crate) fn cost_for(u: &crate::sessions::parse::Usage, p: ModelPrice) -> f64 {
    let input_cost = u.input_tokens as f64 * p.input_per_mtok / 1_000_000.0;
    let output_cost = u.output_tokens as f64 * p.output_per_mtok / 1_000_000.0;
    let cache_read_cost = u.cache_read.unwrap_or(0) as f64
        * p.input_per_mtok * p.cache_read_multiplier
        / 1_000_000.0;
    let cache_write_cost = u.cache_write.unwrap_or(0) as f64
        * p.input_per_mtok * p.cache_write_multiplier
        / 1_000_000.0;
    input_cost + output_cost + cache_read_cost + cache_write_cost
}

fn round_usd(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::parse::{Conversation, SessionRecord, Usage};

    fn conv_with(records: Vec<SessionRecord>) -> Conversation {
        Conversation { session_id: "x".into(), records }
    }

    fn assistant(model: &str, usage: Usage) -> SessionRecord {
        SessionRecord::Assistant {
            content: String::new(),
            model: Some(model.into()),
            ts: None,
            usage: Some(usage),
        }
    }

    #[test]
    fn price_for_known_families() {
        let opus = price_for("claude-opus-4-1-20250514");
        assert_eq!(opus.input_per_mtok, 15.0);
        assert_eq!(opus.output_per_mtok, 75.0);

        let sonnet = price_for("claude-sonnet-4-5-20250929");
        assert_eq!(sonnet.input_per_mtok, 3.0);
        assert_eq!(sonnet.output_per_mtok, 15.0);

        let haiku = price_for("claude-haiku-4-5");
        assert_eq!(haiku.input_per_mtok, 1.0);
        assert_eq!(haiku.output_per_mtok, 5.0);
    }

    #[test]
    fn price_for_unknown_falls_back_to_sonnet() {
        let p = price_for("gpt-4o");
        assert_eq!(p.input_per_mtok, 3.0);
        assert!(!is_known_model("gpt-4o"));
        assert!(is_known_model("claude-sonnet-4-5"));
    }

    #[test]
    fn cost_for_sonnet_basic() {
        let u = Usage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read: None,
            cache_write: None,
        };
        let c = cost_for(&u, price_for("claude-sonnet-4-5"));
        // 1M input @ $3 + 100k output @ $15 = $3 + $1.50 = $4.50
        assert!((c - 4.5).abs() < 1e-9, "got {c}");
    }

    #[test]
    fn cost_for_cache_reads_and_writes() {
        let u = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read: Some(1_000_000),
            cache_write: Some(1_000_000),
        };
        let c = cost_for(&u, price_for("claude-opus-4-1"));
        // cache_read: 1M * $15 * 0.1 = $1.50
        // cache_write: 1M * $15 * 1.25 = $18.75
        // total: $20.25
        assert!((c - 20.25).abs() < 1e-9, "got {c}");
    }

    #[test]
    fn compute_aggregates_per_model() {
        let conv = conv_with(vec![
            assistant("claude-sonnet-4-5", Usage {
                input_tokens: 500_000, output_tokens: 50_000,
                cache_read: Some(200_000), cache_write: Some(100_000),
            }),
            assistant("claude-sonnet-4-5", Usage {
                input_tokens: 100_000, output_tokens: 10_000,
                cache_read: None, cache_write: None,
            }),
            // Heavier Opus run so Opus dominates the cost-sorted list.
            assistant("claude-opus-4-1", Usage {
                input_tokens: 2_000_000, output_tokens: 100_000,
                cache_read: None, cache_write: None,
            }),
        ]);
        let b = compute(&conv).unwrap();
        assert_eq!(b.per_model.len(), 2);
        // Sorted by cost desc — Opus at $30+ / Sonnet at $3.
        assert_eq!(b.per_model[0].model, "claude-opus-4-1");
        assert_eq!(b.per_model[1].model, "claude-sonnet-4-5");
        // Sonnet totals: 600k input, 60k output, 200k cache_read, 100k cache_write.
        let s = &b.per_model[1];
        assert_eq!(s.input_tokens, 600_000);
        assert_eq!(s.output_tokens, 60_000);
        assert_eq!(s.cache_read, 200_000);
        assert_eq!(s.cache_write, 100_000);
        // Opus totals: 2M input, 100k output.
        let o = &b.per_model[0];
        assert_eq!(o.input_tokens, 2_000_000);
        assert_eq!(o.output_tokens, 100_000);
        assert_eq!(b.total_input_tokens, 2_600_000);
        assert_eq!(b.total_output_tokens, 160_000);
        assert_eq!(b.total_cache_read, 200_000);
        assert_eq!(b.total_cache_write, 100_000);
        assert_eq!(b.estimated_records, 0);
    }

    #[test]
    fn compute_marks_unknown_model_as_estimated() {
        let conv = conv_with(vec![
            assistant("gpt-4o", Usage {
                input_tokens: 1000, output_tokens: 100,
                cache_read: None, cache_write: None,
            }),
        ]);
        let b = compute(&conv).unwrap();
        assert_eq!(b.per_model.len(), 1);
        assert_eq!(b.per_model[0].model, "gpt-4o");
        assert_eq!(b.estimated_records, 1);
    }

    #[test]
    fn compute_ignores_records_without_usage() {
        let conv = conv_with(vec![
            SessionRecord::Assistant {
                content: "x".into(),
                model: Some("claude-sonnet-4-5".into()),
                ts: None,
                usage: None,
            },
            assistant("claude-sonnet-4-5", Usage {
                input_tokens: 1000, output_tokens: 100,
                cache_read: None, cache_write: None,
            }),
        ]);
        let b = compute(&conv).unwrap();
        assert_eq!(b.total_input_tokens, 1000);
        assert_eq!(b.per_model.len(), 1);
    }

    #[test]
    fn compute_empty_conversation_yields_zero_breakdown() {
        let b = compute(&conv_with(vec![])).unwrap();
        assert_eq!(b.total_input_tokens, 0);
        assert!(b.per_model.is_empty());
        assert_eq!(b.estimated_cost_usd, 0.0);
        assert_eq!(b.estimated_records, 0);
    }

    #[test]
    fn compute_handles_assistant_with_no_model() {
        let conv = conv_with(vec![
            SessionRecord::Assistant {
                content: "x".into(),
                model: None,
                ts: None,
                usage: Some(Usage {
                    input_tokens: 1000,
                    output_tokens: 100,
                    cache_read: None,
                    cache_write: None,
                }),
            },
        ]);
        let b = compute(&conv).unwrap();
        assert_eq!(b.per_model.len(), 1);
        assert_eq!(b.per_model[0].model, "unknown");
        // Unknown model + no recognizable name → marked estimated.
        assert_eq!(b.estimated_records, 1);
    }
}