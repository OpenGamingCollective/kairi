//! Counts the votes on a mailing list thread, using the rules the OGC voted for
mod ballot;
mod motion;
mod policy;
mod roster;
mod runoff;
mod tally;

pub use ballot::{Ballot, Reason, Rejected, Reply, Vote};
pub use motion::{Choice, Motion, MotionError};
pub use policy::Policy;
pub use roster::{Entity, Roster};
pub use runoff::{Outcome, Round};
pub use tally::{Standing, Tally, tally};
