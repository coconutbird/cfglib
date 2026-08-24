macro_rules! emit_or_break {
    ($on_event:expr, $event:expr) => {
        if let core::ops::ControlFlow::Break(value) = $on_event($event) {
            return Some(value);
        }
    };
}

mod breadth_first;
mod depth_first;

pub use breadth_first::{BfsEvent, breadth_first_events};
pub use depth_first::{DfsEvent, depth_first_events};

#[cfg(test)]
mod tests;
