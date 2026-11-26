//! Counts the votes on a mailing list thread, using the rules the OGC voted for
mod ballot;
mod motion;
mod roster;
mod tally;

pub use ballot::{Vote, read as read_ballot};
pub use motion::{Choice, Motion, MotionError};
pub use roster::{Entity, Roster};
pub use tally::{Ballot, Outcome, Policy, Reason, Rejected, Reply, Round, Standing, Tally, tally};
