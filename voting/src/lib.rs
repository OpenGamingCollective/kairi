//! Counts the votes on a mailing list thread, using the rules the OGC voted for
mod motion;
mod roster;

pub use motion::{Choice, Motion, MotionError};
pub use roster::{Entity, Roster, UNAFFILIATED};
