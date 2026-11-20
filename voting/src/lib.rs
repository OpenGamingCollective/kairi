//! Counts the votes of a mailing list thread, the way the OGC voted to count them
mod ballot;
mod motion;
mod roster;
mod tally;

pub use ballot::{Reading, Vote, read as read_ballot};
pub use motion::{Choice, Motion, MotionError};
pub use roster::{Entity, Roster, UNAFFILIATED};
pub use tally::{Ballot, Outcome, Policy, Reason, Rejected, Reply, Round, Standing, Tally, tally};
