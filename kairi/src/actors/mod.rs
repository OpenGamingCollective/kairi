mod discord;
mod public_inbox_mirror;
mod public_inbox_reader;
mod supervisor;
mod voting;

pub use public_inbox_reader::{FetchList, PublicInboxReader};
pub use supervisor::{Kairi, KairiArg};
pub use voting::{Count, CountVotes, Voting};

#[derive(Debug, thiserror::Error)]
#[error("{0} isn't running")]
pub struct NotRunning(&'static str);

/// lookup voting actor
pub fn voting() -> Result<kameo::actor::ActorRef<Voting>, NotRunning> {
    lookup()
}

/// lookup reader actor
pub fn reader() -> Result<kameo::actor::ActorRef<PublicInboxReader>, NotRunning> {
    lookup()
}

/// lookup actor by type
fn lookup<A: kameo::Actor>() -> Result<kameo::actor::ActorRef<A>, NotRunning> {
    kameo::actor::ActorRef::<A>::lookup(A::name())
        .ok()
        .flatten()
        .ok_or(NotRunning(A::name()))
}
