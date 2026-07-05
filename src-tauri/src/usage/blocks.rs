//! Plan 14 — Claude 5-hour billing-window reconstruction.
//!
//! Ported from ccusage (MIT © ryoppippi) `identify_session_blocks` and its
//! Rust port ccstat (MIT). All timestamps are epoch-millis UTC. A block
//! starts at the first entry floored to the top of the hour; a new block
//! opens when an entry is more than `SESSION_MS` past the block start OR
//! past the previous entry (strict `>`); a block's end (its reset time) is
//! `start + SESSION_MS`.

/// The Claude billing window: 5 hours, in milliseconds.
pub const SESSION_MS: i64 = 5 * 60 * 60 * 1000; // 18_000_000

/// Floor an epoch-millis timestamp to the top of its UTC hour.
pub fn floor_to_hour(ms: i64) -> i64 {
    ms.div_euclid(3_600_000) * 3_600_000
}

/// One reconstructed 5-hour block. `end_ms` is the reset time shown to the user.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo {
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_active: bool,
}

/// Identify all 5-hour blocks from ascending-sorted event timestamps.
pub fn identify_blocks(sorted_ts: &[i64]) -> Vec<BlockInfo> {
    let mut blocks = Vec::new();
    let mut start: Option<i64> = None;
    let mut last: i64 = 0;
    for &ts in sorted_ts {
        match start {
            None => start = Some(floor_to_hour(ts)),
            Some(s) => {
                if ts - s > SESSION_MS || ts - last > SESSION_MS {
                    blocks.push(BlockInfo { start_ms: s, end_ms: s + SESSION_MS, is_active: false });
                    start = Some(floor_to_hour(ts));
                }
            }
        }
        last = ts;
    }
    if let Some(s) = start {
        blocks.push(BlockInfo { start_ms: s, end_ms: s + SESSION_MS, is_active: false });
    }
    blocks
}

/// The current block = the most recent block, marked active iff the last
/// entry is within `SESSION_MS` of `now` AND `now` is before the block end.
pub fn current_block(sorted_ts: &[i64], now_ms: i64) -> Option<BlockInfo> {
    let last_ts = *sorted_ts.last()?;
    let mut b = identify_blocks(sorted_ts).pop()?;
    b.is_active = (now_ms - last_ts) < SESSION_MS && now_ms < b.end_ms;
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    const HOUR: i64 = 3_600_000;

    #[test]
    fn floor_to_hour_snaps_down() {
        assert_eq!(floor_to_hour(10 * HOUR + 500_000), 10 * HOUR);
        assert_eq!(floor_to_hour(10 * HOUR), 10 * HOUR);
    }

    #[test]
    fn single_entry_one_block_end_is_start_plus_5h() {
        let t = 10 * HOUR + 30 * 60 * 1000; // 10:30 → floors to 10:00
        let blocks = identify_blocks(&[t]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_ms, 10 * HOUR);
        assert_eq!(blocks[0].end_ms, 10 * HOUR + SESSION_MS);
    }

    #[test]
    fn gap_over_5h_opens_new_block() {
        let a = 10 * HOUR;
        let b = a + SESSION_MS + 1; // >5h after previous entry
        let blocks = identify_blocks(&[a, b]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].start_ms, floor_to_hour(a));
        assert_eq!(blocks[1].start_ms, floor_to_hour(b));
    }

    #[test]
    fn entries_within_5h_stay_one_block() {
        let a = 10 * HOUR;
        let b = a + 4 * HOUR; // within 5h of start and previous
        let blocks = identify_blocks(&[a, b]);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn boundary_exactly_5h_is_same_block() {
        // strict `>`: exactly SESSION_MS after start is NOT a new block
        let a = 10 * HOUR;
        let b = a + SESSION_MS;
        assert_eq!(identify_blocks(&[a, b]).len(), 1);
    }

    #[test]
    fn current_block_active_when_recent_and_before_end() {
        let start = 10 * HOUR;
        let last = start + 60 * 60 * 1000; // 1h in
        let now = last + 30 * 60 * 1000;   // 30m after last, still < end
        let b = current_block(&[start, last], now).unwrap();
        assert!(b.is_active);
        assert_eq!(b.end_ms, start + SESSION_MS);
    }

    #[test]
    fn current_block_inactive_when_stale() {
        let start = 10 * HOUR;
        let last = start + 60 * 60 * 1000;
        let now = last + SESSION_MS + 1; // >5h since last activity
        let b = current_block(&[start, last], now).unwrap();
        assert!(!b.is_active);
    }

    #[test]
    fn current_block_none_when_empty() {
        assert!(current_block(&[], 123).is_none());
    }
}
