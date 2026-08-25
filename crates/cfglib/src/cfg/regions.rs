//! Exception-region accessors of [`Cfg`].

use crate::block::BlockId;
use crate::region::{Handler, HandlerRef, Region, RegionId};

use super::Cfg;

impl<I, E> Cfg<I, E> {
    /// All exception-handler regions.
    #[inline]
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Returns one exception region by its stable identity.
    #[must_use]
    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.get(id.index())
    }

    /// Returns mutable access to one exception region by its stable identity.
    ///
    /// The region identity itself remains owned by this CFG; callers should not
    /// replace [`Region::id`] with a different value.
    pub fn region_mut(&mut self, id: RegionId) -> Option<&mut Region> {
        self.regions.get_mut(id.index())
    }

    /// Returns one handler by its stable region-and-position identity.
    #[must_use]
    pub fn handler(&self, handler: HandlerRef) -> Option<&Handler> {
        self.region(handler.region())?.handlers.get(handler.index())
    }

    /// Returns mutable access to one handler by its stable identity.
    pub fn handler_mut(&mut self, handler: HandlerRef) -> Option<&mut Handler> {
        self.region_mut(handler.region())?
            .handlers
            .get_mut(handler.index())
    }

    /// Add a region and return its id.
    pub fn add_region(&mut self, mut region: Region) -> RegionId {
        let id = RegionId::from_index(self.regions.len());
        region.id = id;
        self.regions.push(region);
        id
    }

    /// Returns the innermost region that protects `block`, if any.
    #[must_use]
    pub fn protecting_region(&self, block: BlockId) -> Option<&Region> {
        // Return the deepest (last-added) region whose protected set
        // contains this block.
        self.regions
            .iter()
            .rev()
            .find(|r| r.protected_blocks.contains(&block))
    }
}
