//! The Solana commitment ladder, ordered.
//!
//! `processed` < `confirmed` < `finalized`. The ordering is meaningful: a higher
//! commitment is a strictly stronger guarantee, so the derived `PartialOrd`/`Ord`
//! (by declaration order) is the real-world ordering.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Commitment {
    /// Included in the most recent block the node knows; may be on a minority
    /// fork; can be dropped.
    Processed,
    /// Voted on by a supermajority (>2/3 stake) - optimistic confirmation.
    Confirmed,
    /// Rooted: >2/3 stake plus 31+ confirmed blocks built on top. Irreversible.
    Finalized,
}

impl Commitment {
    /// 0 = processed, 1 = confirmed, 2 = finalized.
    pub fn rank(self) -> u8 {
        match self {
            Commitment::Processed => 0,
            Commitment::Confirmed => 1,
            Commitment::Finalized => 2,
        }
    }

    /// True if `self` is at least as strong as `other`.
    pub fn at_least(self, other: Commitment) -> bool {
        self >= other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_strength() {
        assert!(Commitment::Confirmed.at_least(Commitment::Processed));
        assert!(Commitment::Finalized.at_least(Commitment::Confirmed));
        assert!(!Commitment::Processed.at_least(Commitment::Confirmed));
        assert_eq!(Commitment::Processed.rank(), 0);
        assert_eq!(Commitment::Finalized.rank(), 2);
    }
}
