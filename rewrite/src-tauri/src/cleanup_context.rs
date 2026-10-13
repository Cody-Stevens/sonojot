//! Round 6 daily context packet: token-aware budgeting and adjacent
//! read-only context for full-dictation and chunked cleanup.
//!
//! This module is cleanup-only. It owns no model process, clipboard, settings
//! or history. All sizes are UTF-8 byte spans into the immutable source; the
//! caller assembles deterministically and never duplicates overlap.
//!
//! Token accounting: the bundled llama-server runs with `--ctx-size 4096`
//! (`--parallel 1`). The authoritative count is the runtime tokenizer
//! (`POST /tokenize`, see `crate::cleanup::Server`), which the caller uses
//! whenever the server is warm. The byte math in this module is a
//! **conservative upper bound only** (`upper_bound_tokens`): it can only
//! over-estimate, so a budget that fits here can never overfill the context,
//! including multilingual input where `ceil(bytes / 4)` would undercount.
//! Never present a byte-derived figure as an exact token count.

/// Effective runtime context for cleanup generation.
pub const CTX_TOKENS: usize = 4096;
/// Cold owned-editable cap per request. Used only when the runtime tokenizer
/// is unavailable; the measured path in `crate::cleanup` may exceed it when
/// the actual token count fits (whole input in one pass). 1200 bytes is the
/// largest cold value whose upper-bound budget (system + owned + output +
/// 512 reserve) still fits 4096 for typical system prompts.
pub const OWNED_BYTES: usize = 1200;
/// Cold read-only neighbour text per side. Clipped by `adjacent_limit_for`
/// to whatever the remaining budget allows (possibly zero); never edited,
/// never duplicated in output.
pub const ADJACENT_BYTES: usize = 400;
/// Output allowance per chunk: owned bytes as an upper-bound token count
/// plus 256 spare, matching the existing `chunk UTF-8 bytes + 256` rule.
/// Bytes upper-bound tokens (~1 token/byte worst case), so this allowance
/// can only over-provision output, never truncate it for length reasons.
pub const OUTPUT_SPARE_TOKENS: usize = 256;
/// Safety reserve for the chat template, sampling overhead and estimation
/// slack. Covers the system/user template expansion (well under 100 tokens
/// for our prompts) with wide margin.
pub const SAFETY_RESERVE_TOKENS: usize = 512;
/// Fixed upper-bound overhead per tokenized piece (BOS/template specials).
pub const PER_PIECE_OVERHEAD_TOKENS: usize = 16;

/// Conservative token upper bound for `bytes_len` UTF-8 bytes. Documented
/// over-estimate, never exact: BPE-style vocabularies emit at most about one
/// token per byte, plus a small fixed overhead. Multilingual text (where
/// `ceil(bytes / 4)` undercounts) stays safely inside this bound.
pub fn upper_bound_tokens(bytes_len: usize) -> usize {
    bytes_len.saturating_add(PER_PIECE_OVERHEAD_TOKENS).max(1)
}

/// Tokens available for owned input after subtracting system, vocabulary,
/// output allowance and reserve. All inputs are upper-bound token counts.
/// Returns 0 when over budget.
pub fn available_input_tokens(
    system_tokens_ub: usize,
    vocabulary_tokens_ub: usize,
    output_allowance_tokens: usize,
) -> usize {
    let used = system_tokens_ub
        .saturating_add(vocabulary_tokens_ub)
        .saturating_add(output_allowance_tokens)
        .saturating_add(SAFETY_RESERVE_TOKENS);
    CTX_TOKENS.saturating_sub(used)
}

/// Max owned bytes that fit the token budget under the conservative upper
/// bound (adjacent context is clipped separately by `adjacent_limit_for`).
/// Solves `owned + output(owned + spare) + system + vocab + reserve <= CTX`
/// in upper-bound space. Never exceeds OWNED_BYTES (cold cap); floors at 256
/// so progress is always possible. The measured path may return larger
/// limits; this stays the safe cold default.
pub fn owned_limit_for(system_bytes: usize, vocabulary_bytes: usize) -> usize {
    let system_ub = upper_bound_tokens(system_bytes);
    let vocab_ub = upper_bound_tokens(vocabulary_bytes);
    // Fixed-point solve: start from the cold cap, shrink until the
    // upper-bound total fits (at most a handful of iterations).
    let mut owned = OWNED_BYTES;
    loop {
        let output = upper_bound_tokens(owned).saturating_add(OUTPUT_SPARE_TOKENS);
        let avail = available_input_tokens(system_ub, vocab_ub, output);
        // Owned upper-bound tokens must fit the available input budget.
        if upper_bound_tokens(owned) <= avail || owned <= 256 {
            break;
        }
        owned = owned.saturating_sub(64).max(256);
    }
    owned.min(OWNED_BYTES).max(256)
}

/// Adjacent read-only bytes per side that still fit after the owned span is
/// budgeted. Clips the 400-byte cold want to the remaining upper-bound
/// budget (possibly zero); the caller snaps to UTF-8 boundaries.
pub fn adjacent_limit_for(
    system_bytes: usize,
    vocabulary_bytes: usize,
    owned_bytes: usize,
) -> usize {
    let output = upper_bound_tokens(owned_bytes).saturating_add(OUTPUT_SPARE_TOKENS);
    let avail = available_input_tokens(
        upper_bound_tokens(system_bytes),
        upper_bound_tokens(vocabulary_bytes),
        output,
    );
    let owned_ub = upper_bound_tokens(owned_bytes);
    let remaining = avail.saturating_sub(owned_ub);
    // Upper-bound space runs ~1 token/byte, so remaining tokens map 1:1 to
    // bytes; split across both sides.
    (remaining / 2).min(ADJACENT_BYTES)
}

/// Cold single-pass check under the upper bound: true when the whole source
/// plus system, vocabulary, output allowance and reserve fits 4096. The
/// measured path (runtime tokenizer) is authoritative when warm and admits
/// strictly more single-pass inputs; this never wrongly claims a fit.
pub fn single_pass_fits_cold(
    system_bytes: usize,
    vocabulary_bytes: usize,
    source_bytes: usize,
) -> bool {
    let output = upper_bound_tokens(source_bytes).saturating_add(OUTPUT_SPARE_TOKENS);
    let avail = available_input_tokens(
        upper_bound_tokens(system_bytes),
        upper_bound_tokens(vocabulary_bytes),
        output,
    );
    upper_bound_tokens(source_bytes) <= avail
}

/// One editable span with read-only neighbours. All ranges are byte offsets
/// into the complete source. `owned` partitions the source without overlap;
/// `left`/`right` are context only and must not appear in output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkPacket {
    pub index: usize,
    pub owned: std::ops::Range<usize>,
    pub left: std::ops::Range<usize>,
    pub right: std::ops::Range<usize>,
}

/// Build prompt text for one chunk. The model must return only the revised
/// owned span. Context sections are labelled data, never instructions.
pub fn build_prompt(
    source: &str,
    packet: &ChunkPacket,
    mode: &str,
    system_hint: &str,
) -> String {
    let owned = &source[packet.owned.clone()];
    if packet.left.is_empty() && packet.right.is_empty() {
        // Single-pass full dictation: no extra sections, least surprise for
        // small models.
        return owned.to_owned();
    }
    let left = &source[packet.left.clone()];
    let right = &source[packet.right.clone()];
    format!(
        "{system_hint}\nMode: {mode}\nOwned span to edit (return only this span revised):\n<owned>\n{owned}\n</owned>\nLeft context (read-only, do not repeat):\n<left>\n{left}\n</left>\nRight context (read-only, do not repeat):\n<right>\n{right}\n</right>"
    )
}

/// Verify owned spans partition the source exactly once with no gaps,
/// overlaps or missing tail bytes.
pub fn verify_partition(source_len: usize, owned: &[std::ops::Range<usize>]) -> Result<(), String> {
    let mut cursor = 0;
    for range in owned {
        if range.start != cursor || range.end <= range.start || range.end > source_len {
            return Err("Chunk partition has a gap, overlap or out-of-bounds span.".into());
        }
        cursor = range.end;
    }
    if cursor != source_len {
        return Err("Chunk partition drops trailing bytes.".into());
    }
    Ok(())
}

/// Check that assembled output preserves every non-retired tail byte.
/// `tail` is the source tail that must survive (e.g. final constraints).
/// Returns Err when the tail is missing.
pub fn check_tail_preserved(final_text: &str, tail: &str) -> Result<(), String> {
    if tail.is_empty() {
        return Ok(());
    }
    if final_text.contains(tail) || final_text.trim_end().ends_with(tail.trim()) {
        Ok(())
    } else {
        Err("Final output drops trailing source text.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_upper_bound_never_undercounts() {
        assert_eq!(upper_bound_tokens(0), 16);
        assert_eq!(upper_bound_tokens(4), 20);
        // Multilingual input that `ceil(bytes/4)` would place at 2 tokens
        // stays inside the bound: 8 bytes can never exceed 24 tokens.
        assert!(upper_bound_tokens(8) >= 8 + 1);
        assert_eq!(upper_bound_tokens(1200), 1216);
        // Cold owned limit still respects the cap and never claims exactness.
        let limit = owned_limit_for(800, 400);
        assert!(limit <= OWNED_BYTES, "{limit}");
        assert!(limit >= 256, "{limit}");
        // Adjacent clips to budget instead of overfilling.
        let adj = adjacent_limit_for(800, 400, limit);
        assert!(adj <= ADJACENT_BYTES, "{adj}");
        // A short dictation fits cold single-pass; a 12 KB one does not.
        assert!(single_pass_fits_cold(800, 0, 512));
        assert!(!single_pass_fits_cold(800, 0, 12_000));
    }

    #[test]
    fn single_pass_prompt_is_bare_owned() {
        let source = "Keep all words.";
        let packet = ChunkPacket {
            index: 0,
            owned: 0..source.len(),
            left: 0..0,
            right: 0..0,
        };
        assert_eq!(build_prompt(source, &packet, "format", "hint"), source);
    }

    #[test]
    fn multi_pass_prompt_labels_owned_and_context() {
        let source = "left-owned-right";
        let packet = ChunkPacket {
            index: 0,
            owned: 5..10,
            left: 0..5,
            right: 10..15,
        };
        let prompt = build_prompt(source, &packet, "clean", "Use context.");
        assert!(prompt.contains("<owned>\nowned\n</owned>"));
        assert!(prompt.contains("<left>"));
        assert!(prompt.contains("read-only"));
        assert!(!prompt.contains("left-owned-right"));
    }

    #[test]
    fn partition_verifier_catches_gaps_and_missing_tail() {
        assert!(verify_partition(10, &[0..5, 5..10]).is_ok());
        assert!(verify_partition(10, &[0..5, 6..10]).is_err());
        assert!(verify_partition(10, &[0..5, 5..9]).is_err());
        assert!(verify_partition(10, &[0..5, 4..10]).is_err());
        assert!(check_tail_preserved("keep do not upload anything", "do not upload anything").is_ok());
        assert!(check_tail_preserved("keep something", "do not upload anything").is_err());
    }
}
