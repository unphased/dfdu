//! Navigation matching and bounded sampled-hash workers.
use dfdu_peer::{Nav, Verdict};

pub(crate) fn mark(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::HashMatch => "✓",
        Verdict::HashMismatch => "✗",
        Verdict::Pending => "…",
        Verdict::NameOnly => "name",
        Verdict::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_marks_are_distinct() {
        assert_eq!(mark(Verdict::HashMatch), "✓");
        assert_eq!(mark(Verdict::HashMismatch), "✗");
        assert_eq!(mark(Verdict::Pending), "…");
    }
    #[test]
    fn stale_hash_cannot_change_candidates() {
        let mut state = State::default();
        state.seq = 2;
        state.complete((1, "old".into(), Some("hash".into())));
        assert!(state.hashes.is_empty());
    }
}
