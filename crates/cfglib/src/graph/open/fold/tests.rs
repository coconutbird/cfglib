extern crate alloc;

use alloc::vec::Vec;
use core::ops::ControlFlow;

use super::{FoldEnter, MarkScope, OpenFold, OpenFoldConfig, open_fold_post_order};

/// A member-lookup fold over a static base table: a node with an `own`
/// answer is a leaf, an unanswered node combines its bases with sticky
/// ambiguity, and every node folds with the given mark scope.
struct Lookup<'a> {
    bases: &'a [&'a [usize]],
    owns: &'a [Option<&'static str>],
    scope: MarkScope,
    folded: Vec<usize>,
}

impl<'a> Lookup<'a> {
    fn new(bases: &'a [&'a [usize]], owns: &'a [Option<&'static str>], scope: MarkScope) -> Self {
        Self {
            bases,
            owns,
            scope,
            folded: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Combined {
    Nothing,
    One(&'static str),
    Ambiguous,
}

impl OpenFold for Lookup<'_> {
    type Node = usize;
    type Mark = usize;
    type Value = &'static str;
    type Accumulator = Combined;

    fn successors(&mut self, node: &usize, out: &mut Vec<usize>) {
        out.extend_from_slice(self.bases[*node]);
    }

    fn mark(&mut self, node: &usize) -> Option<usize> {
        Some(*node)
    }

    fn enter(&mut self, node: &usize) -> FoldEnter<&'static str, Combined> {
        self.folded.push(*node);
        match self.owns[*node] {
            Some(name) => FoldEnter::Leaf(Some(name)),
            None => FoldEnter::Fold {
                accumulator: Combined::Nothing,
                marks: self.scope,
            },
        }
    }

    fn absorb(
        &mut self,
        accumulator: &mut Combined,
        _child: &usize,
        value: Option<&'static str>,
    ) -> ControlFlow<()> {
        if let Some(value) = value {
            *accumulator = match accumulator {
                Combined::Nothing => Combined::One(value),
                Combined::One(_) | Combined::Ambiguous => Combined::Ambiguous,
            };
        }
        ControlFlow::Continue(())
    }

    fn finish(&mut self, _node: &usize, accumulator: Combined) -> Option<&'static str> {
        match accumulator {
            Combined::One(value) => Some(value),
            Combined::Nothing | Combined::Ambiguous => None,
        }
    }
}

#[test]
fn a_chain_propagates_the_deep_answer() {
    let bases: &[&[usize]] = &[&[1], &[2], &[]];
    let owns = &[None, None, Some("deep")];
    let mut lookup = Lookup::new(bases, owns, MarkScope::Isolated);
    assert_eq!(
        open_fold_post_order(&mut lookup, 0, OpenFoldConfig::new()),
        Some("deep"),
    );
}

#[test]
fn isolated_marks_fold_a_diamond_base_once_per_route() {
    // 0 derives 1 and 2; both derive 3, which declares the member. Per-path
    // marks fold 3 twice, so 0 sees two yielding bases: ambiguous.
    let bases: &[&[usize]] = &[&[1, 2], &[3], &[3], &[]];
    let owns = &[None, None, None, Some("member")];
    let mut lookup = Lookup::new(bases, owns, MarkScope::Isolated);
    assert_eq!(
        open_fold_post_order(&mut lookup, 0, OpenFoldConfig::new()),
        None,
    );
    assert_eq!(
        lookup.folded,
        [0, 1, 3, 2, 3],
        "the shared base folds once per route"
    );
}

#[test]
fn an_own_declaration_hides_the_ambiguous_bases() {
    // 1 declares the member itself, so its diamond below never folds and 0
    // receives exactly one yielding base.
    let bases: &[&[usize]] = &[&[1], &[2, 3], &[4], &[4], &[]];
    let owns = &[None, Some("hidden"), None, None, Some("member")];
    let mut lookup = Lookup::new(bases, owns, MarkScope::Isolated);
    assert_eq!(
        open_fold_post_order(&mut lookup, 0, OpenFoldConfig::new()),
        Some("hidden"),
    );
    assert_eq!(lookup.folded, [0, 1], "the leaf's bases are never visited");
}

#[test]
fn shared_marks_refuse_the_second_route() {
    // The same diamond under persistent marks: 3 folds once, the second
    // route absorbs `None`, and the lookup resolves unambiguously.
    let bases: &[&[usize]] = &[&[1, 2], &[3], &[3], &[]];
    let owns = &[None, None, None, Some("member")];
    let mut lookup = Lookup::new(bases, owns, MarkScope::Shared);
    assert_eq!(
        open_fold_post_order(&mut lookup, 0, OpenFoldConfig::new()),
        Some("member"),
    );
    assert_eq!(
        lookup.folded,
        [0, 1, 3, 2],
        "the marked base never re-folds"
    );
}

#[test]
fn cycles_contribute_nothing_on_their_route() {
    // 0 -> 1 -> 0 with the answer on a side branch of 1.
    let bases: &[&[usize]] = &[&[1], &[0, 2], &[]];
    let owns = &[None, None, Some("side")];
    let mut lookup = Lookup::new(bases, owns, MarkScope::Isolated);
    assert_eq!(
        open_fold_post_order(&mut lookup, 0, OpenFoldConfig::new()),
        Some("side"),
    );
}

#[test]
fn the_depth_bound_refuses_deeper_children() {
    let bases: &[&[usize]] = &[&[1], &[2], &[]];
    let owns = &[None, None, Some("deep")];
    let mut shallow = Lookup::new(bases, owns, MarkScope::Isolated);
    assert_eq!(
        open_fold_post_order(&mut shallow, 0, OpenFoldConfig::new().with_max_depth(1)),
        None,
    );
    let mut exact = Lookup::new(bases, owns, MarkScope::Isolated);
    assert_eq!(
        open_fold_post_order(&mut exact, 0, OpenFoldConfig::new().with_max_depth(2)),
        Some("deep"),
    );
}

/// A first-match fold: the first yielding child wins and stops its
/// siblings, mirroring a base-chain lookup that returns the first hit.
struct FirstMatch<'a> {
    children: &'a [&'a [usize]],
    owns: &'a [Option<&'static str>],
    folded: Vec<usize>,
}

impl OpenFold for FirstMatch<'_> {
    type Node = usize;
    type Mark = usize;
    type Value = &'static str;
    type Accumulator = Option<&'static str>;

    fn successors(&mut self, node: &usize, out: &mut Vec<usize>) {
        out.extend_from_slice(self.children[*node]);
    }

    fn mark(&mut self, node: &usize) -> Option<usize> {
        Some(*node)
    }

    fn enter(&mut self, node: &usize) -> FoldEnter<&'static str, Option<&'static str>> {
        self.folded.push(*node);
        match self.owns[*node] {
            Some(name) => FoldEnter::Leaf(Some(name)),
            None => FoldEnter::Fold {
                accumulator: None,
                marks: MarkScope::Shared,
            },
        }
    }

    fn absorb(
        &mut self,
        accumulator: &mut Option<&'static str>,
        _child: &usize,
        value: Option<&'static str>,
    ) -> ControlFlow<()> {
        if value.is_some() {
            *accumulator = value;
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn finish(&mut self, _node: &usize, accumulator: Option<&'static str>) -> Option<&'static str> {
        accumulator
    }
}

#[test]
fn a_breaking_absorb_skips_the_remaining_children() {
    let children: &[&[usize]] = &[&[1, 2, 3], &[], &[], &[]];
    let owns = &[None, Some("first"), Some("second"), Some("third")];
    let mut lookup = FirstMatch {
        children,
        owns,
        folded: Vec::new(),
    };
    assert_eq!(
        open_fold_post_order(&mut lookup, 0, OpenFoldConfig::new()),
        Some("first"),
    );
    assert_eq!(lookup.folded, [0, 1], "later siblings are never entered");
}

/// A fold whose nodes carry no marks at all: revisits are allowed, so a
/// shared subterm is evaluated once per reference, like a value algebra.
struct Unmarked<'a> {
    children: &'a [&'a [usize]],
    folded: Vec<usize>,
}

impl OpenFold for Unmarked<'_> {
    type Node = usize;
    type Mark = usize;
    type Value = usize;
    type Accumulator = usize;

    fn successors(&mut self, node: &usize, out: &mut Vec<usize>) {
        out.extend_from_slice(self.children[*node]);
    }

    fn mark(&mut self, _node: &usize) -> Option<usize> {
        None
    }

    fn enter(&mut self, node: &usize) -> FoldEnter<usize, usize> {
        self.folded.push(*node);
        if self.children[*node].is_empty() {
            FoldEnter::Leaf(Some(1))
        } else {
            FoldEnter::Fold {
                accumulator: 0,
                marks: MarkScope::Shared,
            }
        }
    }

    fn absorb(
        &mut self,
        accumulator: &mut usize,
        _child: &usize,
        value: Option<usize>,
    ) -> ControlFlow<()> {
        *accumulator += value.unwrap_or_default();
        ControlFlow::Continue(())
    }

    fn finish(&mut self, _node: &usize, accumulator: usize) -> Option<usize> {
        Some(accumulator)
    }
}

#[test]
fn unmarked_nodes_fold_on_every_reach() {
    // Both 1 and 2 share leaf 3; without marks it counts once per route.
    let children: &[&[usize]] = &[&[1, 2], &[3], &[3], &[]];
    let mut fold = Unmarked {
        children,
        folded: Vec::new(),
    };
    assert_eq!(
        open_fold_post_order(&mut fold, 0, OpenFoldConfig::new()),
        Some(2),
    );
    assert_eq!(fold.folded, [0, 1, 3, 2, 3]);
}
