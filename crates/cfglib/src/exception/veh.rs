//! Process-wide vectored exception and continue handlers.

extern crate alloc;

use alloc::vec::Vec;

/// Opaque identity returned when a vectored handler is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectoredHandlerId(u64);

impl VectoredHandlerId {
    /// Construct an identity from a serialized/raw value.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the serialized/raw identity.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Which Windows vectored-handler list owns a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectoredHandlerKind {
    /// Handler called before frame-based exception dispatch.
    Exception,
    /// Handler considered when execution may continue.
    Continue,
}

/// Requested position when registering a vectored handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectoredHandlerOrder {
    /// Insert at the front of the selected list.
    First,
    /// Append at the back of the selected list.
    Last,
}

/// One process-wide vectored handler registration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectoredHandler<H> {
    /// Opaque registration identity.
    pub id: VectoredHandlerId,
    /// Exception or continue list.
    pub kind: VectoredHandlerKind,
    /// Consumer-owned function/address identity.
    pub handler: H,
}

/// Ordered process-wide Windows vectored-handler lists.
///
/// This model intentionally does not attach handlers to a [`Cfg`](crate::Cfg):
/// VEH registrations are dynamic and not frame- or protected-region-based.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectoredExceptionModel<H> {
    next_id: u64,
    exception_handlers: Vec<VectoredHandler<H>>,
    continue_handlers: Vec<VectoredHandler<H>>,
}

impl<H> VectoredExceptionModel<H> {
    /// Construct empty exception and continue handler lists.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_id: 0,
            exception_handlers: Vec::new(),
            continue_handlers: Vec::new(),
        }
    }

    /// Register a handler and return its opaque identity.
    ///
    /// A later `First` registration precedes every earlier registration in
    /// the same list, matching `AddVectoredExceptionHandler` semantics.
    ///
    /// # Panics
    ///
    /// Panics after all `u64` registration identities have been consumed.
    pub fn register(
        &mut self,
        kind: VectoredHandlerKind,
        order: VectoredHandlerOrder,
        handler: H,
    ) -> VectoredHandlerId {
        let id = VectoredHandlerId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("vectored handler identity space exhausted");
        let registration = VectoredHandler { id, kind, handler };
        let handlers = match kind {
            VectoredHandlerKind::Exception => &mut self.exception_handlers,
            VectoredHandlerKind::Continue => &mut self.continue_handlers,
        };
        match order {
            VectoredHandlerOrder::First => handlers.insert(0, registration),
            VectoredHandlerOrder::Last => handlers.push(registration),
        }
        id
    }

    /// Remove a registration from either vectored-handler list.
    pub fn remove(&mut self, id: VectoredHandlerId) -> Option<VectoredHandler<H>> {
        for handlers in [&mut self.exception_handlers, &mut self.continue_handlers] {
            if let Some(index) = handlers.iter().position(|handler| handler.id == id) {
                return Some(handlers.remove(index));
            }
        }
        None
    }

    /// Registrations of `kind` in call order.
    pub fn handlers(&self, kind: VectoredHandlerKind) -> core::slice::Iter<'_, VectoredHandler<H>> {
        match kind {
            VectoredHandlerKind::Exception => self.exception_handlers.iter(),
            VectoredHandlerKind::Continue => self.continue_handlers.iter(),
        }
    }

    /// Total number of vectored exception and continue registrations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.exception_handlers.len() + self.continue_handlers.len()
    }

    /// Whether both vectored-handler lists are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exception_handlers.is_empty() && self.continue_handlers.is_empty()
    }
}

impl<H> Default for VectoredExceptionModel<H> {
    fn default() -> Self {
        Self::new()
    }
}

/// Concise alias for [`VectoredExceptionModel`].
pub type VehModel<H> = VectoredExceptionModel<H>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_last_and_removal_follow_windows_registration_order() {
        let mut model = VehModel::new();
        let tail = model.register(
            VectoredHandlerKind::Exception,
            VectoredHandlerOrder::Last,
            "tail",
        );
        model.register(
            VectoredHandlerKind::Exception,
            VectoredHandlerOrder::First,
            "first-a",
        );
        model.register(
            VectoredHandlerKind::Exception,
            VectoredHandlerOrder::First,
            "first-b",
        );
        model.register(
            VectoredHandlerKind::Continue,
            VectoredHandlerOrder::Last,
            "continue",
        );

        assert_eq!(
            model
                .handlers(VectoredHandlerKind::Exception)
                .map(|registration| registration.handler)
                .collect::<Vec<_>>(),
            alloc::vec!["first-b", "first-a", "tail"]
        );
        assert_eq!(model.remove(tail).map(|entry| entry.handler), Some("tail"));
        assert_eq!(model.len(), 3);
    }
}
