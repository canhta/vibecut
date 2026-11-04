//! One cumulative run budget shared by ASR and suggestion invocations.
use crate::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    limit: u64,
    reserved: u64,
}

impl Budget {
    pub fn new(limit: u64) -> Self {
        Self { limit, reserved: 0 }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn reserved(&self) -> u64 {
        self.reserved
    }

    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.reserved)
    }

    /// Reserve an invocation's upper-bound estimate before any upload. The
    /// reservation stays spent even if the invocation later fails.
    pub fn reserve(&mut self, estimate: u64) -> Result<()> {
        let next = self.reserved.saturating_add(estimate);
        if next > self.limit {
            return Err(Error::CostCap {
                estimate: next,
                cap: self.limit,
            });
        }
        self.reserved = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_asr_and_llm_reservations_share_one_cap() {
        let mut budget = Budget::new(350_000);
        budget.reserve(110_062).unwrap();
        assert_eq!(budget.remaining(), 239_938);
        let error = budget.reserve(344_064).unwrap_err();
        assert!(matches!(
            error,
            Error::CostCap {
                estimate: 454_126,
                cap: 350_000
            }
        ));
    }

    #[test]
    fn reservations_accumulate_across_invocations() {
        let mut budget = Budget::new(700_000);
        budget.reserve(344_064).unwrap();
        budget.reserve(344_064).unwrap();
        assert_eq!(budget.reserved(), 688_128);
        assert!(budget.reserve(1).is_ok());
        assert_eq!(budget.remaining(), 11_871);
    }
}
