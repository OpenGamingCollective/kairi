//! Counts the votes of a mailing list thread, the way the OGC voted to count them
mod motion;
mod roster;

pub use motion::{Choice, Motion, MotionError};
pub use roster::{Entity, Roster, UNAFFILIATED};
