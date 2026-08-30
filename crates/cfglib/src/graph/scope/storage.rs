//! Owned scope-graph storage and dense identities.

extern crate alloc;

use alloc::vec::Vec;
use core::ops::Index;

use crate::graph::directed::{DirectedGraph, EdgeId as DirectedEdgeId, NodeId};
use crate::graph::edge_view::{DenseEdgeId, EdgeGraphView, EdgeRef};
use crate::graph::view::{DenseNodeId, DirectedGraphView};

/// Dense identity of a scope in a [`ScopeGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeId(u32);

impl ScopeId {
    /// Construct a scope identity from a dense zero-based index.
    ///
    /// # Panics
    ///
    /// Panics when `index` exceeds `u32::MAX`.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("scope index exceeds u32::MAX"))
    }

    /// Construct a scope identity from its raw integer representation.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the dense zero-based index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    const fn graph_id(self) -> NodeId {
        NodeId::from_raw(self.0)
    }
}

impl core::fmt::Display for ScopeId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "s{}", self.0)
    }
}

impl DenseNodeId for ScopeId {
    fn from_index(index: usize) -> Self {
        Self::from_index(index)
    }

    fn index(self) -> usize {
        self.index()
    }
}

/// Stable identity of a labeled edge in a [`ScopeGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeEdgeId(u32);

impl ScopeEdgeId {
    /// Construct an edge identity from a dense arena index.
    ///
    /// # Panics
    ///
    /// Panics when `index` exceeds `u32::MAX`.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("scope-edge index exceeds u32::MAX"))
    }

    /// Construct an edge identity from its raw integer representation.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the dense arena index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    const fn graph_id(self) -> DirectedEdgeId {
        DirectedEdgeId::from_raw(self.0)
    }
}

impl core::fmt::Display for ScopeEdgeId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "se{}", self.0)
    }
}

impl DenseEdgeId for ScopeEdgeId {
    fn from_index(index: usize) -> Self {
        Self::from_index(index)
    }

    fn index(self) -> usize {
        self.index()
    }
}

/// Stable identity of relation-tagged data in a [`ScopeGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeDatumId(u32);

impl ScopeDatumId {
    /// Construct a datum identity from a dense zero-based index.
    ///
    /// # Panics
    ///
    /// Panics when `index` exceeds `u32::MAX`.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("scope datum index exceeds u32::MAX"))
    }

    /// Construct a datum identity from its raw integer representation.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the dense zero-based index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for ScopeDatumId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "d{}", self.0)
    }
}

/// Stable identity of a reference in a [`ScopeGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeReferenceId(u32);

impl ScopeReferenceId {
    /// Construct a reference identity from a dense zero-based index.
    ///
    /// # Panics
    ///
    /// Panics when `index` exceeds `u32::MAX`.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("scope reference index exceeds u32::MAX"))
    }

    /// Construct a reference identity from its raw integer representation.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the dense zero-based index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for ScopeReferenceId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "r{}", self.0)
    }
}

/// One scope carrying consumer-defined metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Scope<S> {
    payload: S,
}

impl<S> Scope<S> {
    /// Borrow the consumer-defined scope payload.
    #[must_use]
    pub const fn payload(&self) -> &S {
        &self.payload
    }

    /// Mutably borrow the consumer-defined scope payload.
    pub const fn payload_mut(&mut self) -> &mut S {
        &mut self.payload
    }

    /// Consume the scope and return its payload.
    #[must_use]
    pub fn into_payload(self) -> S {
        self.payload
    }
}

/// One relation-tagged datum owned by a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeDatum<R, D> {
    scope: ScopeId,
    relation: R,
    data: D,
}

impl<R, D> ScopeDatum<R, D> {
    /// The scope that owns this datum.
    #[must_use]
    pub const fn scope(&self) -> ScopeId {
        self.scope
    }

    /// Borrow the consumer-defined relation tag.
    #[must_use]
    pub const fn relation(&self) -> &R {
        &self.relation
    }

    /// Borrow the consumer-defined data.
    #[must_use]
    pub const fn data(&self) -> &D {
        &self.data
    }

    /// Mutably borrow the consumer-defined data.
    pub const fn data_mut(&mut self) -> &mut D {
        &mut self.data
    }

    /// Consume the datum and return its relation and data.
    #[must_use]
    pub fn into_parts(self) -> (R, D) {
        (self.relation, self.data)
    }
}

/// One reference whose lookup starts in a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeReference<Q> {
    scope: ScopeId,
    data: Q,
}

impl<Q> ScopeReference<Q> {
    /// The scope where lookup starts.
    #[must_use]
    pub const fn scope(&self) -> ScopeId {
        self.scope
    }

    /// Borrow the consumer-defined reference data.
    #[must_use]
    pub const fn data(&self) -> &Q {
        &self.data
    }

    /// Mutably borrow the consumer-defined reference data.
    pub const fn data_mut(&mut self) -> &mut Q {
        &mut self.data
    }

    /// Consume the reference and return its data.
    #[must_use]
    pub fn into_data(self) -> Q {
        self.data
    }
}

/// Owned, language-parametric scope-graph storage.
///
/// Edges are directed from the scope where a query is running to a scope whose
/// data becomes reachable. Their labels remain entirely consumer-defined.
/// Data carry a separate relation tag so one graph can hold value, type,
/// member, label, macro, or other namespaces without parallel graph stores.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeGraph<S = (), L = (), R = (), D = (), Q = ()> {
    graph: DirectedGraph<Scope<S>, L>,
    data: Vec<ScopeDatum<R, D>>,
    references: Vec<ScopeReference<Q>>,
    scope_data: Vec<Vec<ScopeDatumId>>,
    scope_references: Vec<Vec<ScopeReferenceId>>,
}

impl<S, L, R, D, Q> ScopeGraph<S, L, R, D, Q> {
    /// Create an empty scope graph.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            graph: DirectedGraph::new(),
            data: Vec::new(),
            references: Vec::new(),
            scope_data: Vec::new(),
            scope_references: Vec::new(),
        }
    }

    /// Create an empty scope graph with the requested capacities.
    #[must_use]
    pub fn with_capacity(scopes: usize, edges: usize, data: usize, references: usize) -> Self {
        Self {
            graph: DirectedGraph::with_capacity(scopes, edges),
            data: Vec::with_capacity(data),
            references: Vec::with_capacity(references),
            scope_data: Vec::with_capacity(scopes),
            scope_references: Vec::with_capacity(scopes),
        }
    }

    /// Add a scope and return its stable identity.
    pub fn add_scope(&mut self, payload: S) -> ScopeId {
        let id = self.graph.add_node(Scope { payload });
        self.scope_data.push(Vec::new());
        self.scope_references.push(Vec::new());
        ScopeId::from_index(id.index())
    }

    /// Add a labeled reachability edge and return its stable identity.
    ///
    /// Parallel and cyclic edges are valid. Resolution paths themselves are
    /// cycle-free, so cyclic import or parent relations remain total.
    ///
    /// # Panics
    ///
    /// Panics when either endpoint does not belong to this graph.
    pub fn add_edge(&mut self, source: ScopeId, target: ScopeId, label: L) -> ScopeEdgeId {
        let id = self
            .graph
            .add_edge(source.graph_id(), target.graph_id(), label);
        ScopeEdgeId::from_index(id.index())
    }

    /// Add relation-tagged data to `scope` and return its stable identity.
    ///
    /// # Panics
    ///
    /// Panics when `scope` does not belong to this graph.
    pub fn add_datum(&mut self, scope: ScopeId, relation: R, data: D) -> ScopeDatumId {
        assert!(scope.index() < self.scope_count(), "scope is out of range");
        let id = ScopeDatumId::from_index(self.data.len());
        self.data.push(ScopeDatum {
            scope,
            relation,
            data,
        });
        self.scope_data[scope.index()].push(id);
        id
    }

    /// Add a reference whose lookup begins in `scope`.
    ///
    /// # Panics
    ///
    /// Panics when `scope` does not belong to this graph.
    pub fn add_reference(&mut self, scope: ScopeId, data: Q) -> ScopeReferenceId {
        assert!(scope.index() < self.scope_count(), "scope is out of range");
        let id = ScopeReferenceId::from_index(self.references.len());
        self.references.push(ScopeReference { scope, data });
        self.scope_references[scope.index()].push(id);
        id
    }

    /// Borrow a scope.
    ///
    /// # Panics
    ///
    /// Panics when `scope` does not belong to this graph.
    #[must_use]
    pub fn scope(&self, scope: ScopeId) -> &Scope<S> {
        self.graph.node(scope.graph_id())
    }

    /// Mutably borrow a scope.
    ///
    /// # Panics
    ///
    /// Panics when `scope` does not belong to this graph.
    pub fn scope_mut(&mut self, scope: ScopeId) -> &mut Scope<S> {
        self.graph.node_mut(scope.graph_id())
    }

    /// Borrow one live edge.
    ///
    /// # Panics
    ///
    /// Panics when `edge` is out of range or was removed.
    #[must_use]
    pub fn edge(&self, edge: ScopeEdgeId) -> EdgeRef<'_, ScopeId, ScopeEdgeId, L> {
        let value = self.graph.edge(edge.graph_id());
        EdgeRef::new(
            edge,
            ScopeId::from_index(value.source().index()),
            ScopeId::from_index(value.target().index()),
            value.payload(),
        )
    }

    /// Mutably borrow a live edge's label.
    ///
    /// # Panics
    ///
    /// Panics when `edge` is out of range or was removed.
    pub fn edge_label_mut(&mut self, edge: ScopeEdgeId) -> &mut L {
        self.graph.edge_mut(edge.graph_id()).payload_mut()
    }

    /// Remove an edge while preserving every other identity.
    pub fn remove_edge(&mut self, edge: ScopeEdgeId) -> Option<L> {
        self.graph
            .remove_edge(edge.graph_id())
            .map(crate::graph::directed::DirectedEdge::into_payload)
    }

    /// Borrow relation-tagged data.
    ///
    /// # Panics
    ///
    /// Panics when `datum` does not belong to this graph.
    #[must_use]
    pub fn datum(&self, datum: ScopeDatumId) -> &ScopeDatum<R, D> {
        &self.data[datum.index()]
    }

    /// Mutably borrow relation-tagged data.
    ///
    /// # Panics
    ///
    /// Panics when `datum` does not belong to this graph.
    pub fn datum_mut(&mut self, datum: ScopeDatumId) -> &mut ScopeDatum<R, D> {
        &mut self.data[datum.index()]
    }

    /// Borrow a reference.
    ///
    /// # Panics
    ///
    /// Panics when `reference` does not belong to this graph.
    #[must_use]
    pub fn reference(&self, reference: ScopeReferenceId) -> &ScopeReference<Q> {
        &self.references[reference.index()]
    }

    /// Mutably borrow a reference.
    ///
    /// # Panics
    ///
    /// Panics when `reference` does not belong to this graph.
    pub fn reference_mut(&mut self, reference: ScopeReferenceId) -> &mut ScopeReference<Q> {
        &mut self.references[reference.index()]
    }

    /// Data identities owned by `scope`, in insertion order.
    ///
    /// # Panics
    ///
    /// Panics when `scope` does not belong to this graph.
    #[must_use]
    pub fn scope_data(&self, scope: ScopeId) -> &[ScopeDatumId] {
        &self.scope_data[scope.index()]
    }

    /// Reference identities owned by `scope`, in insertion order.
    ///
    /// # Panics
    ///
    /// Panics when `scope` does not belong to this graph.
    #[must_use]
    pub fn scope_references(&self, scope: ScopeId) -> &[ScopeReferenceId] {
        &self.scope_references[scope.index()]
    }

    /// Iterate over every scope identity in allocation order.
    pub fn scope_ids(&self) -> impl ExactSizeIterator<Item = ScopeId> + '_ {
        (0..self.scope_count()).map(ScopeId::from_index)
    }

    /// Iterate over every live edge identity in insertion order.
    pub fn edge_ids(&self) -> impl Iterator<Item = ScopeEdgeId> + '_ {
        self.graph
            .edges()
            .map(|edge| ScopeEdgeId::from_index(edge.id().index()))
    }

    /// Iterate over every datum identity in insertion order.
    pub fn datum_ids(&self) -> impl ExactSizeIterator<Item = ScopeDatumId> + '_ {
        (0..self.data.len()).map(ScopeDatumId::from_index)
    }

    /// Iterate over every reference identity in insertion order.
    pub fn reference_ids(&self) -> impl ExactSizeIterator<Item = ScopeReferenceId> + '_ {
        (0..self.references.len()).map(ScopeReferenceId::from_index)
    }

    /// Return outgoing edge identities in insertion order.
    #[must_use = "iterators are lazy and do nothing unless consumed"]
    pub fn outgoing_edges(
        &self,
        scope: ScopeId,
    ) -> impl ExactSizeIterator<Item = ScopeEdgeId> + '_ {
        self.graph
            .outgoing_edges(scope.graph_id())
            .iter()
            .map(|edge| ScopeEdgeId::from_index(edge.index()))
    }

    /// Return incoming edge identities in insertion order.
    #[must_use = "iterators are lazy and do nothing unless consumed"]
    pub fn incoming_edges(
        &self,
        scope: ScopeId,
    ) -> impl ExactSizeIterator<Item = ScopeEdgeId> + '_ {
        self.graph
            .incoming_edges(scope.graph_id())
            .iter()
            .map(|edge| ScopeEdgeId::from_index(edge.index()))
    }

    /// Return the number of scopes.
    #[must_use]
    pub fn scope_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Return the number of live scope edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Return the number of edge slots, including removed-edge tombstones.
    #[must_use]
    pub fn edge_slot_count(&self) -> usize {
        self.graph.edge_slot_count()
    }

    /// Return the number of relation-tagged data entries.
    #[must_use]
    pub fn datum_count(&self) -> usize {
        self.data.len()
    }

    /// Return the number of references.
    #[must_use]
    pub fn reference_count(&self) -> usize {
        self.references.len()
    }

    /// Return whether the graph contains no scopes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.graph.is_empty()
    }
}

impl<S, L, R, D, Q> Default for ScopeGraph<S, L, R, D, Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, L, R, D, Q> DirectedGraphView for ScopeGraph<S, L, R, D, Q> {
    type NodeId = ScopeId;

    fn node_count(&self) -> usize {
        self.scope_count()
    }

    fn successors(&self, node: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
        self.outgoing_edges(node)
            .map(|edge| self.edge(edge).target())
    }

    fn predecessors(&self, node: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
        self.incoming_edges(node)
            .map(|edge| self.edge(edge).source())
    }
}

impl<S, L, R, D, Q> EdgeGraphView for ScopeGraph<S, L, R, D, Q> {
    type EdgeId = ScopeEdgeId;
    type EdgeData = L;

    fn edge_slot_count(&self) -> usize {
        self.edge_slot_count()
    }

    fn edge_ids(&self) -> impl Iterator<Item = ScopeEdgeId> + '_ {
        self.edge_ids()
    }

    fn outgoing_edges(&self, node: ScopeId) -> impl Iterator<Item = ScopeEdgeId> + '_ {
        self.outgoing_edges(node)
    }

    fn incoming_edges(&self, node: ScopeId) -> impl Iterator<Item = ScopeEdgeId> + '_ {
        self.incoming_edges(node)
    }

    fn edge_ref(&self, edge: ScopeEdgeId) -> EdgeRef<'_, ScopeId, ScopeEdgeId, L> {
        self.edge(edge)
    }
}

impl<S, L, R, D, Q> Index<ScopeId> for ScopeGraph<S, L, R, D, Q> {
    type Output = Scope<S>;

    fn index(&self, scope: ScopeId) -> &Self::Output {
        self.scope(scope)
    }
}

impl<S, L, R, D, Q> Index<ScopeDatumId> for ScopeGraph<S, L, R, D, Q> {
    type Output = ScopeDatum<R, D>;

    fn index(&self, datum: ScopeDatumId) -> &Self::Output {
        self.datum(datum)
    }
}

impl<S, L, R, D, Q> Index<ScopeReferenceId> for ScopeGraph<S, L, R, D, Q> {
    type Output = ScopeReference<Q>;

    fn index(&self, reference: ScopeReferenceId) -> &Self::Output {
        self.reference(reference)
    }
}
