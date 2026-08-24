//! Runtime-neutral exception metadata and normalized platform adapters.
//!
//! Frame-scoped CLR and Windows SEH constructs lower into
//! [`Region`](crate::Region). Dynamic x86 SEH registration and process-wide
//! vectored handlers remain separate models because neither is a lexical CFG
//! region. Platform-specific records stay consumer-owned generic payloads.

mod clr;
mod flow;
mod seh;
mod veh;

pub use clr::{ClrExceptionRegion, ClrHandler, ClrHandlerKind, install_clr_region};
pub use flow::{ExceptionDisposition, ExceptionFlow, ExceptionPhase};
pub use seh::{
    SehExceptionRegion, SehHandler, SehHandlerKind, SehRegistration, SehRegistrationChain,
    install_seh_region,
};
pub use veh::{
    VectoredExceptionModel, VectoredHandler, VectoredHandlerId, VectoredHandlerKind,
    VectoredHandlerOrder, VehModel,
};
