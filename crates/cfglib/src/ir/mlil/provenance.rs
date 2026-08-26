//! MLIL specialization of the shared source-provenance storage.

use super::EntityId;

/// One source span mapped to one MLIL entity.
pub type ProvenanceEntry<D> = crate::ir::provenance::ProvenanceEntry<D, EntityId>;

/// Deterministic many-to-many source-to-MLIL provenance.
pub type ProvenanceMap<D> = crate::ir::provenance::ProvenanceMap<D, EntityId>;
