//! Dense bit-set lattice elements.
//!
//! Set-of-dense-indices facts (visible files, reachable definitions, owned
//! slots) keep re-implementing the same `Vec<bool>` row with a hand-written
//! changed-tracking union. [`DenseBits`] is that row packed 64 indices per
//! word: `Clone + PartialEq` so it drops into any solver's `Fact`, with
//! [`union_with`](DenseBits::union_with) as the join and its change report
//! driving fixpoint convergence checks.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// A fixed-universe set of dense indices, packed 64 per word.
///
/// The universe size is set at construction; every operation stays within
/// it. Use as a dataflow fact with union as the join:
///
/// ```rust
/// use cfglib::DenseBits;
///
/// let mut visible = DenseBits::new(130);
/// assert!(visible.insert(0));
/// assert!(visible.insert(129));
/// let mut merged = DenseBits::new(130);
/// assert!(merged.union_with(&visible));
/// assert!(!merged.union_with(&visible), "a second union changes nothing");
/// assert_eq!(merged.ones().collect::<Vec<_>>(), vec![0, 129]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DenseBits {
    words: Vec<u64>,
    len: usize,
}

impl DenseBits {
    /// Creates the empty set over the universe `0..len`.
    #[must_use]
    pub fn new(len: usize) -> Self {
        Self {
            words: vec![0; len.div_ceil(64)],
            len,
        }
    }

    /// The universe size the set was created with.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the universe is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether `index` is in the set.
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside the universe.
    #[must_use]
    pub fn get(&self, index: usize) -> bool {
        assert!(
            index < self.len,
            "index {index} outside universe {}",
            self.len
        );
        self.words[index / 64] & (1 << (index % 64)) != 0
    }

    /// Inserts `index`, returning whether it was newly set.
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside the universe.
    pub fn insert(&mut self, index: usize) -> bool {
        assert!(
            index < self.len,
            "index {index} outside universe {}",
            self.len
        );
        let word = &mut self.words[index / 64];
        let bit = 1 << (index % 64);
        let newly = *word & bit == 0;
        *word |= bit;
        newly
    }

    /// Unions `other` into this set, returning whether anything changed.
    ///
    /// This is the lattice join: a fixpoint transfer can report convergence
    /// directly from the return value.
    ///
    /// # Panics
    ///
    /// Panics when the universes differ.
    pub fn union_with(&mut self, other: &Self) -> bool {
        assert_eq!(self.len, other.len, "unioned sets must share one universe");
        let mut changed = false;
        for (word, &other_word) in self.words.iter_mut().zip(&other.words) {
            let merged = *word | other_word;
            changed |= merged != *word;
            *word = merged;
        }
        changed
    }

    /// The number of indices in the set.
    #[must_use]
    pub fn count_ones(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// The set's indices in ascending order.
    pub fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(position, &word)| {
            (0..64)
                .filter(move |bit| word & (1 << bit) != 0)
                .map(move |bit| position * 64 + bit)
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::vec::Vec;

    use super::DenseBits;

    #[test]
    fn insert_get_and_count_agree() {
        let mut bits = DenseBits::new(70);
        assert!(!bits.get(63));
        assert!(bits.insert(63));
        assert!(!bits.insert(63), "a second insert is a no-op");
        assert!(bits.insert(64));
        assert!(bits.get(63));
        assert!(bits.get(64));
        assert!(!bits.get(0));
        assert_eq!(bits.count_ones(), 2);
        assert_eq!(bits.ones().collect::<Vec<_>>(), [63, 64]);
    }

    #[test]
    fn union_reports_change_exactly_when_bits_arrive() {
        let mut left = DenseBits::new(10);
        left.insert(1);
        let mut right = DenseBits::new(10);
        right.insert(1);
        right.insert(9);
        assert!(left.union_with(&right));
        assert!(!left.union_with(&right));
        assert_eq!(left.ones().collect::<Vec<_>>(), [1, 9]);
    }

    #[test]
    fn equality_is_set_equality() {
        let mut left = DenseBits::new(5);
        let mut right = DenseBits::new(5);
        left.insert(2);
        assert_ne!(left, right);
        right.insert(2);
        assert_eq!(left, right);
    }
}
