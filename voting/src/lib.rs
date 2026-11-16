//! Counts the votes on a mailing list thread, using the rules the OGC voted for
mod ballot;
mod motion;
mod roster;

pub use ballot::{Reading, Vote, read as read_ballot};
pub use motion::{Choice, Motion, MotionError};
pub use roster::{Entity, Roster, UNAFFILIATED};
