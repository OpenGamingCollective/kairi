//! Counts a vote the way the membership approved it, with square-root weighting and a
//! dual-gate quorum:
//! <https://lists.opengamingcollective.org/governance/2ba6fc23-d466-44ec-b472-4d558ec9797d@gmail.com/>
//!
//! - Each organisation's weight is the square root of its roster size. The unaffiliated vote
//!   as one block
//! - Internal quorum: an organisation only counts if enough of its members voted
//! - Coalition quorum: the vote is void unless enough entities count
//! - Weights are normalised across the entities that count, then split among their ballots
//! - An option passes with more than half of the counted weight. Rankings go to a runoff
//!
//! Things the approved text leaves open, mostly settled by the [`Policy`]:
//!
//! - The unaffiliated block has no internal quorum, and weighs the root of its voter count
//! - A member of several organisations votes in the first one by slug
//! - Abstentions and exhausted ballots count in the majority
//! - Options tied for last are eliminated together
//! - The last two options still run off

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    ballot::{self, Reading, Vote},
    motion::{Motion, MotionError},
    roster::Roster,
};

/// Scores closer than this are tied
const EPSILON: f64 = 1e-9;

/// What the unaffiliated block's size is based on
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockWeight {
    /// Only the members who voted
    #[default]
    Voters,
    /// Every unaffiliated member, as the approved text reads
    Roster,
}

/// How abstentions count
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Abstentions {
    /// Count toward both the quorum and the majority
    #[default]
    Counted,
    /// Count toward the quorum only
    QuorumOnly,
    /// Treated as if the member didn't vote
    Ignored,
}

/// What happens to a ballot once all its options are eliminated
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Exhausted {
    /// It still counts toward the majority
    #[default]
    Counted,
    /// It no longer counts toward the majority
    Dropped,
}

/// What to do with two options left and no majority
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LastTwo {
    /// Eliminate the weaker one and move its ballots to the other
    #[default]
    Runoff,
    /// Stop there, with no majority
    Final,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Fraction of an organisation's members who must vote for it to count (0 to 1)
    pub internal_quorum: f64,
    /// Fraction of the entities that must count for the vote to be valid (0 to 1)
    pub coalition_quorum: f64,
    /// How long a vote stays open if its first mail doesn't say
    pub window: TimeDelta,
    pub unaffiliated_weight: BlockWeight,
    pub abstentions: Abstentions,
    pub exhausted: Exhausted,
    pub last_two: LastTwo,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window: TimeDelta::hours(24),
            unaffiliated_weight: BlockWeight::default(),
            abstentions: Abstentions::default(),
            exhausted: Exhausted::default(),
            last_two: LastTwo::default(),
        }
    }
}

/// A reply in the thread
#[derive(Debug, Clone)]
pub struct Reply {
    /// Display name
    pub author: String,
    /// Sender address
    pub sender: Option<String>,
    pub date: DateTime<Utc>,
    pub url: String,
    pub body: String,
}

/// A member's vote. If they voted more than once, the last one
#[derive(Debug, Clone, PartialEq)]
pub struct Ballot {
    /// Username on the roster
    pub member: String,
    pub vote: Vote,
    pub url: String,
    /// Whether its entity reached quorum
    pub counted: bool,
}

/// A reply that looks like a vote but isn't counted
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    pub author: String,
    pub url: String,
    pub reason: Reason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The sender isn't a member
    UnknownSender,
    /// Sent before the vote opened or after it closed
    OutsideWindow,
    /// A member who never wrote a readable vote
    NoVote,
    Conflicting,
    UnknownOption(u32),
}

/// An organisation or the unaffiliated block
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub slug: String,
    pub name: String,
    pub unaffiliated: bool,
    pub roster: usize,
    /// Roster size, or the number of voters for the unaffiliated block
    pub size: usize,
    pub voted: usize,
    /// Usernames of its voters
    pub voters: Vec<String>,
    /// Whether it passed the internal quorum
    pub counts: bool,
    /// Its share of the vote, or 0 if it doesn't count
    pub weight: f64,
}

/// One round of the instant runoff
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    /// Share behind each option still in the race, in motion order
    pub scores: Vec<(u32, f64)>,
    /// Options eliminated after this round
    pub eliminated: Vec<u32>,
    /// The total an option needs more than half of to win
    pub total: f64,
}

impl Round {
    /// Weight behind the options still in the race
    pub fn voted(&self) -> f64 {
        self.scores.iter().map(|(_, score)| score).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed(u32),
    /// No option has more than half of the weight that counts
    NoMajority,
    Tied,
    /// The coalition quorum wasn't met
    Void,
    /// The quorum was met, but nobody voted for any option
    NoVotes,
}

#[derive(Debug, Clone)]
pub struct Tally {
    pub motion: Motion,
    /// Still open, so the outcome can change
    pub open: bool,
    /// Members on the roster, and how many of them can vote
    pub members: usize,
    pub eligible: usize,
    pub ballots: Vec<Ballot>,
    pub rejected: Vec<Rejected>,
    pub standings: Vec<Standing>,
    pub coalition_quorum: f64,
    /// Empty when the vote is void
    pub rounds: Vec<Round>,
    /// Share of the vote that abstained
    pub abstained: f64,
    pub outcome: Outcome,
}

impl Tally {
    /// How many organisations and blocks count
    pub fn counting(&self) -> usize {
        self.standings
            .iter()
            .filter(|standing| standing.counts)
            .count()
    }
}

/// Count the votes on a thread as of `now`. `mail` is the first mail's body, `replies` the rest
pub fn tally(
    mail: &str,
    sent: DateTime<Utc>,
    replies: &[Reply],
    roster: &Roster,
    policy: &Policy,
    now: DateTime<Utc>,
) -> Result<Tally, MotionError> {
    let motion = Motion::read(mail, sent, policy.window)?;
    let (ballots, rejected) = collect(&motion, replies, roster);

    // An ignored abstention is treated as no vote
    let voted = |member: &String| {
        ballots.get(member).is_some_and(|ballot| {
            policy.abstentions != Abstentions::Ignored || ballot.vote != Vote::Abstain
        })
    };

    let entities = roster.entities();
    let mut standings: Vec<Standing> = entities
        .iter()
        .map(|entity| {
            let roster = entity.members.len();
            let voters: Vec<String> = entity
                .members
                .iter()
                .filter(|member| voted(member))
                .cloned()
                .collect();
            let voted = voters.len();
            let voters_only =
                entity.unaffiliated && policy.unaffiliated_weight == BlockWeight::Voters;
            let counts = voted > 0
                && (entity.unaffiliated || voted as f64 >= policy.internal_quorum * roster as f64);
            Standing {
                slug: entity.slug.clone(),
                name: entity.name.clone(),
                unaffiliated: entity.unaffiliated,
                roster,
                size: if voters_only { voted } else { roster },
                voted,
                voters,
                counts,
                weight: 0.0,
            }
        })
        .collect();

    let pool: f64 = standings
        .iter()
        .filter(|standing| standing.counts)
        .map(|standing| (standing.size as f64).sqrt())
        .sum();
    for standing in standings.iter_mut().filter(|standing| standing.counts) {
        standing.weight = (standing.size as f64).sqrt() / pool;
    }

    let counting = standings.iter().filter(|standing| standing.counts).count();
    let void =
        counting == 0 || (counting as f64) < policy.coalition_quorum * standings.len() as f64;

    let mut shares: BTreeMap<&str, f64> = BTreeMap::new();
    for standing in standings.iter().filter(|standing| standing.counts) {
        let share = standing.weight / standing.voted as f64;
        for member in &standing.voters {
            shares.insert(member.as_str(), share);
        }
    }

    let mut abstained = 0.0;
    let mut ranked: Vec<(&[u32], f64)> = Vec::new();
    for (member, share) in &shares {
        match &ballots[*member].vote {
            Vote::Abstain => abstained += share,
            Vote::Ranked(options) => ranked.push((options, *share)),
        }
    }

    let options: Vec<u32> = motion.options.iter().map(|option| option.number).collect();
    let (rounds, outcome) = if void {
        (Vec::new(), Outcome::Void)
    } else {
        let abstained = match policy.abstentions {
            Abstentions::Counted => abstained,
            Abstentions::QuorumOnly | Abstentions::Ignored => 0.0,
        };
        runoff(
            &options,
            &ranked,
            abstained,
            policy.exhausted,
            policy.last_two,
        )
    };

    let mut ballots: Vec<Ballot> = ballots
        .into_iter()
        .map(|(member, ballot)| Ballot {
            counted: shares.contains_key(&*member),
            ..ballot
        })
        .collect();
    ballots.sort_by(|a, b| a.member.cmp(&b.member));

    Ok(Tally {
        open: now < motion.closes,
        motion,
        members: roster.members(),
        eligible: roster.eligible(),
        ballots,
        rejected,
        standings,
        coalition_quorum: policy.coalition_quorum,
        rounds,
        abstained: if void { 0.0 } else { abstained },
        outcome,
    })
}

/// Each member's last readable vote, and the rejected replies
fn collect(
    motion: &Motion,
    replies: &[Reply],
    roster: &Roster,
) -> (BTreeMap<String, Ballot>, Vec<Rejected>) {
    let mut replies: Vec<&Reply> = replies.iter().collect();
    replies.sort_by_key(|reply| reply.date);

    let mut ballots: BTreeMap<String, Ballot> = BTreeMap::new();
    let mut rejected = Vec::new();
    // Replied without voting. Only reported if they never vote
    let mut silent: BTreeMap<String, &Reply> = BTreeMap::new();

    for reply in replies {
        let reading = ballot::read(&reply.body, motion);
        let member = reply
            .sender
            .as_deref()
            .and_then(|sender| roster.member_of(sender));
        let reject = |reason| Rejected {
            author: reply.author.clone(),
            url: reply.url.clone(),
            reason,
        };

        let open = (motion.opens..=motion.closes).contains(&reply.date);
        let (Some(member), true) = (member, open) else {
            // Anyone can comment. Only a readable vote is worth reporting
            if matches!(reading, Reading::Vote(_)) {
                rejected.push(reject(match member {
                    Some(_) => Reason::OutsideWindow,
                    None => Reason::UnknownSender,
                }));
            }
            continue;
        };

        match reading {
            Reading::Vote(vote) => {
                ballots.insert(
                    member.to_string(),
                    Ballot {
                        member: member.to_string(),
                        vote,
                        url: reply.url.clone(),
                        counted: false,
                    },
                );
            }
            Reading::NoVote => {
                silent.entry(member.to_string()).or_insert(reply);
            }
            Reading::Conflicting => rejected.push(reject(Reason::Conflicting)),
            Reading::UnknownOption(option) => rejected.push(reject(Reason::UnknownOption(option))),
        }
    }

    for (member, reply) in silent {
        if !ballots.contains_key(&member) {
            rejected.push(Rejected {
                author: reply.author.clone(),
                url: reply.url.clone(),
                reason: Reason::NoVote,
            });
        }
    }
    (ballots, rejected)
}

/// Eliminate the weakest options until one has more than half of the counted weight,
/// including `abstained`
fn runoff(
    options: &[u32],
    ballots: &[(&[u32], f64)],
    abstained: f64,
    exhausted: Exhausted,
    last_two: LastTwo,
) -> (Vec<Round>, Outcome) {
    let mut running: Vec<u32> = options.to_vec();
    let mut rounds = Vec::new();
    let ranked: f64 = ballots.iter().map(|(_, share)| share).sum();

    loop {
        let mut scores: Vec<(u32, f64)> = running.iter().map(|option| (*option, 0.0)).collect();
        for (ranking, share) in ballots {
            // Every option on this ballot is out
            let Some(choice) = ranking.iter().find(|option| running.contains(option)) else {
                continue;
            };
            if let Some(score) = scores.iter_mut().find(|score| score.0 == *choice) {
                score.1 += share;
            }
        }

        let voted: f64 = scores.iter().map(|(_, score)| score).sum();
        let total = abstained
            + match exhausted {
                Exhausted::Counted => ranked,
                Exhausted::Dropped => voted,
            };
        let winner = scores
            .iter()
            .find(|(_, score)| *score > total / 2.0 + EPSILON)
            .map(|(option, _)| *option);
        // The last remaining option only gets a round if it passes
        if winner.is_none() && scores.len() == 1 && !rounds.is_empty() {
            return (rounds, Outcome::NoMajority);
        }

        let lowest = scores
            .iter()
            .map(|(_, score)| *score)
            .fold(f64::MAX, f64::min);
        // Options tied for last are eliminated together
        let eliminated: Vec<u32> = scores
            .iter()
            .filter(|(_, score)| winner.is_none() && *score <= lowest + EPSILON)
            .map(|(option, _)| *option)
            .collect();

        let stuck = eliminated.len() == scores.len();
        // `LastTwo::Final` stops without eliminating
        let last = last_two == LastTwo::Final && winner.is_none() && scores.len() <= 2;
        rounds.push(Round {
            scores,
            eliminated: if stuck || last {
                Vec::new()
            } else {
                eliminated.clone()
            },
            total,
        });

        if let Some(winner) = winner {
            return (rounds, Outcome::Passed(winner));
        }
        if voted <= EPSILON {
            return (rounds, Outcome::NoVotes);
        }
        if stuck {
            return (rounds, Outcome::Tied);
        }
        if last {
            return (rounds, Outcome::NoMajority);
        }
        running.retain(|option| !eliminated.contains(option));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{motion::tests::sent, roster::UNAFFILIATED};

    const MAIL: &str = "Voting will be open for 24 hours.\nOption 1: a\nOption 2: b\n";
    const THREE: &str = "Option 1: a\nOption 2: b\nOption 3: c\n";

    /// `big` has 4 members, `duo` 2, `solo` 1, 4 are unaffiliated and `mute` has no email
    fn roster() -> Roster {
        let members: Vec<String> = [
            "big1", "big2", "big3", "big4", "duo1", "duo2", "solo1", "free1", "free2", "free3",
            "mute",
        ]
        .iter()
        .map(|name| {
            let email = if *name == "mute" {
                String::new()
            } else {
                format!("{name}@example.org")
            };
            format!(r#""{name}": {{ "email": "{email}" }}"#)
        })
        .collect();
        Roster::parse(&format!(
            r#"{{
                "members": {{ {} }},
                "groups": {{
                    "big": {{ "org": true, "members": ["big1", "big2", "big3", "big4"] }},
                    "duo": {{ "org": true, "members": ["duo1", "duo2"] }},
                    "solo": {{ "org": true, "members": ["solo1", "big1"] }},
                    "project": {{ "org": false, "members": ["free1"] }}
                }}
            }}"#,
            members.join(",")
        ))
        .unwrap()
    }

    fn reply(member: &str, hours: i64, body: &str) -> Reply {
        Reply {
            author: member.to_string(),
            sender: Some(format!("{member}@example.org")),
            date: sent() + TimeDelta::hours(hours),
            url: format!("https://l/{member}/{hours}/"),
            body: body.to_string(),
        }
    }

    fn count_with(mail: &str, replies: &[Reply], policy: &Policy) -> Tally {
        let now = sent() + TimeDelta::days(7);
        tally(mail, sent(), replies, &roster(), policy, now).unwrap()
    }

    fn count(replies: &[Reply]) -> Tally {
        count_with(MAIL, replies, &Policy::default())
    }

    fn standing<'a>(tally: &'a Tally, slug: &str) -> &'a Standing {
        tally
            .standings
            .iter()
            .find(|standing| standing.slug == slug)
            .unwrap()
    }

    fn score(tally: &Tally, round: usize, option: u32) -> f64 {
        let round = &tally.rounds[round];
        let (_, score) = round
            .scores
            .iter()
            .find(|(number, _)| *number == option)
            .unwrap();
        *score
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Each voter votes for the option given next to their name
    fn votes(votes: &[(&str, &str)]) -> Vec<Reply> {
        votes
            .iter()
            .map(|(member, body)| reply(member, 1, body))
            .collect()
    }

    #[test]
    fn weights_are_the_square_root_of_the_rosters() {
        let tally = count(&votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 1"),
            ("duo1", "Option 2"),
            ("free1", "Option 2"),
        ]));

        // big 2, duo √2, unaffiliated 1 for its only voter. solo also lists big1, but big1 only
        // votes in big
        let pool = 2.0 + 2f64.sqrt() + 1.0;
        assert!(close(standing(&tally, "big").weight, 2.0 / pool));
        assert!(close(standing(&tally, "duo").weight, 2f64.sqrt() / pool));
        let solo = standing(&tally, "solo");
        assert_eq!((solo.roster, solo.voted, solo.counts), (1, 0, false));
        assert!(close(standing(&tally, UNAFFILIATED).weight, 1.0 / pool));
        // mute can't vote, so the block only counts the three who can
        assert_eq!(standing(&tally, UNAFFILIATED).roster, 3);

        assert!(close(score(&tally, 0, 1), 2.0 / pool));
        assert_eq!(tally.outcome, Outcome::Passed(2));
        assert_eq!((tally.members, tally.eligible), (11, 10));
    }

    #[test]
    fn an_organisation_shares_its_weight_between_its_ballots() {
        let tally = count(&votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 2"),
            ("big3", "Option 2"),
            ("duo1", "Option 1"),
            ("duo2", "Option 1"),
        ]));

        let pool = 2.0 + 2f64.sqrt();
        let first = 2.0 / 3.0 + 2f64.sqrt();
        assert!(close(score(&tally, 0, 1), first / pool));
        assert!(close(score(&tally, 0, 2), (4.0 / 3.0) / pool));
    }

    #[test]
    fn an_organisation_under_its_internal_quorum_doesnt_count() {
        let tally = count(&votes(&[
            ("big2", "Option 1"),
            ("duo1", "Option 2"),
            ("free1", "Option 2"),
        ]));

        let big = standing(&tally, "big");
        assert_eq!((big.voted, big.roster, big.counts), (1, 4, false));
        assert_eq!(big.weight, 0.0);
        assert!(standing(&tally, "duo").counts);
        // The unaffiliated block has no quorum
        assert!(standing(&tally, UNAFFILIATED).counts);

        assert!(close(score(&tally, 0, 2), 1.0));
        assert_eq!(tally.outcome, Outcome::Passed(2));
        let big2 = tally
            .ballots
            .iter()
            .find(|ballot| ballot.member == "big2")
            .unwrap();
        assert!(!big2.counted);
    }

    #[test]
    fn the_vote_is_void_under_the_coalition_quorum() {
        let tally = count(&votes(&[("duo1", "Option 1")]));
        assert_eq!(tally.counting(), 1);
        assert_eq!(tally.outcome, Outcome::Void);
        assert!(tally.rounds.is_empty());

        assert_eq!(count(&[]).outcome, Outcome::Void);

        let lax = Policy {
            coalition_quorum: 0.25,
            ..Policy::default()
        };
        let tally = count_with(MAIL, &votes(&[("duo1", "Option 1")]), &lax);
        assert_eq!(tally.outcome, Outcome::Passed(1));
    }

    #[test]
    fn half_of_the_weight_is_a_tie() {
        let tally = count(&votes(&[
            ("duo1", "Option 1"),
            ("duo2", "Option 2"),
            ("free1", "Option 1"),
            ("free2", "Option 2"),
        ]));
        assert_eq!(tally.outcome, Outcome::Tied);
        assert!(tally.rounds[0].eliminated.is_empty());
    }

    #[test]
    fn abstentions_are_part_of_the_majority() {
        // duo √2: half abstains, half to 1. unaffiliated 1 abstains
        let tally = count(&votes(&[
            ("duo1", "Abstain"),
            ("duo2", "Option 1"),
            ("free1", "abstain"),
        ]));
        assert!(standing(&tally, "duo").counts);
        let pool = 2f64.sqrt() + 1.0;
        assert!(close(score(&tally, 0, 1), 2f64.sqrt() / 2.0 / pool));
        assert!(close(tally.abstained, (2f64.sqrt() / 2.0 + 1.0) / pool));
        // The only option anyone voted for falls short of half
        assert_eq!(tally.outcome, Outcome::NoMajority);
        assert_eq!(tally.rounds.len(), 1);
        assert_eq!(tally.rounds[0].eliminated, [2]);

        // big 2 to 1, duo √2 abstains
        let tally = count(&votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 1"),
            ("duo1", "Abstain"),
        ]));
        assert!(close(score(&tally, 0, 1), 2.0 / (2.0 + 2f64.sqrt())));
        assert_eq!(tally.outcome, Outcome::Passed(1));

        let tally = count(&votes(&[("duo1", "Abstain"), ("free1", "Abstain")]));
        assert_eq!(tally.outcome, Outcome::NoVotes);
    }

    #[test]
    fn the_ballots_of_an_option_that_is_out_can_pass_the_other() {
        // big 2 and solo 1 to 1, duo √2 to 2 then 1, unaffiliated √3 abstains
        let replies = |duo: &str| {
            votes(&[
                ("big1", "Option 1"),
                ("big2", "Option 1"),
                ("solo1", "Option 1"),
                ("duo1", duo),
                ("duo2", duo),
                ("free1", "Abstain"),
                ("free2", "Abstain"),
                ("free3", "Abstain"),
            ])
        };

        // 1 has 3 of 3 + √2 + √3, short of half until it picks up the ballots of 2
        let pool = 3.0 + 2f64.sqrt() + 3f64.sqrt();
        let tally = count(&replies("Option 2 > Option 1"));
        assert_eq!(tally.rounds.len(), 2);
        assert_eq!(tally.rounds[0].eliminated, [2]);
        assert!(close(score(&tally, 1, 1), (3.0 + 2f64.sqrt()) / pool));
        assert_eq!(tally.outcome, Outcome::Passed(1));

        // Nobody who voted for 2 ranked 1
        let tally = count(&replies("Option 2"));
        assert_eq!(tally.rounds.len(), 1);
        assert!(close(score(&tally, 0, 1), 3.0 / pool));
        assert_eq!(tally.outcome, Outcome::NoMajority);
    }

    #[test]
    fn the_last_vote_of_a_member_counts() {
        let tally = count(&[
            reply("duo1", 1, "Option 1"),
            reply("free1", 3, "Option 2"),
            reply("free1", 2, "Option 1"),
            reply("duo1", 4, "Actually, let me explain why."),
        ]);
        let vote = |member: &str| {
            let ballot = tally.ballots.iter().find(|ballot| ballot.member == member);
            ballot.unwrap().vote.clone()
        };
        assert_eq!(vote("free1"), Vote::Ranked(vec![2]));
        // A comment doesn't take a vote back
        assert_eq!(vote("duo1"), Vote::Ranked(vec![1]));
        assert!(tally.rejected.is_empty());
    }

    #[test]
    fn replies_that_arent_counted_say_why() {
        let mut stranger = reply("mallory", 1, "Option 2");
        stranger.author = "Mallory".to_string();
        let mut anonymous = reply("nobody", 1, "Option 2");
        anonymous.sender = None;

        let tally = count(&[
            stranger,
            anonymous,
            reply("mallory", 2, "Just a comment from a stranger"),
            reply("duo1", 25, "Option 2"),
            reply("duo2", -1, "Option 2"),
            reply("big1", 1, "Option 1\nOption 2"),
            reply("big2", 1, "Option 9"),
            reply("big3", 1, "I like turtles"),
            reply("free1", 30, "Thanks for counting!"),
        ]);

        let reasons: Vec<(&str, Reason)> = tally
            .rejected
            .iter()
            .map(|rejected| (&*rejected.author, rejected.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                ("duo2", Reason::OutsideWindow),
                ("Mallory", Reason::UnknownSender),
                ("nobody", Reason::UnknownSender),
                ("big1", Reason::Conflicting),
                ("big2", Reason::UnknownOption(9)),
                ("duo1", Reason::OutsideWindow),
                ("big3", Reason::NoVote),
            ]
        );
        assert!(tally.ballots.is_empty());
        assert_eq!(tally.rejected[1].url, "https://l/mallory/1/");
    }

    #[test]
    fn the_count_posted_after_the_vote_is_not_a_late_vote() {
        let results =
            "Results after 24h\n\nOption 1: 2/2 votes (100 %)\nOption 2: 0/2 votes (0%)\n";
        let tally = count(&[reply("duo1", 1, "Option 1"), reply("duo1", 26, results)]);
        assert!(tally.rejected.is_empty());
        assert_eq!(tally.ballots.len(), 1);
    }

    #[test]
    fn the_vote_closes_when_the_mail_says() {
        let on_time = [reply("duo1", 24, "Option 1"), reply("free1", 0, "Option 1")];
        assert_eq!(count(&on_time).ballots.len(), 2);

        let now = sent() + TimeDelta::hours(23);
        let open = tally(MAIL, sent(), &[], &roster(), &Policy::default(), now).unwrap();
        assert!(open.open);
        assert!(!count(&[]).open);

        // The mail doesn't say
        let long = Policy {
            window: TimeDelta::hours(72),
            ..Policy::default()
        };
        let late = [reply("duo1", 48, "Option 1 > Option 2")];
        assert_eq!(count_with(THREE, &late, &long).ballots.len(), 1);
        assert!(
            count_with(THREE, &late, &Policy::default())
                .ballots
                .is_empty()
        );
    }

    #[test]
    fn rankings_are_an_instant_runoff() {
        let tally = count_with(
            THREE,
            &votes(&[
                ("duo1", "Option 1 > Option 2"),
                ("duo2", "Option 3 > Option 2 > Option 1"),
                ("free1", "Option 2 > Option 3"),
                ("free2", "Option 2"),
                ("solo1", "Option 3 > Option 2"),
                ("big1", "Option 3"),
            ]),
            &Policy::default(),
        );

        // duo √2: half to 1, half to 3. unaffiliated √2 to 2. solo 1 to 3
        assert_eq!(tally.rounds.len(), 2);
        assert_eq!(tally.rounds[0].eliminated, [1]);
        assert_eq!(tally.rounds[1].scores.len(), 2);
        // duo1's ballot moves to option 2
        let pool = 2f64.sqrt() + 2f64.sqrt() + 1.0;
        assert!(close(
            score(&tally, 1, 2),
            (2f64.sqrt() + 2f64.sqrt() / 2.0) / pool
        ));
        assert_eq!(tally.outcome, Outcome::Passed(2));
    }

    #[test]
    fn ballots_out_of_options_are_part_of_the_majority() {
        let tally = count_with(
            THREE,
            &votes(&[
                ("duo1", "Option 1"),
                ("duo2", "Option 2"),
                ("free1", "Option 3 > Option 1"),
                ("free2", "Option 2"),
                ("solo1", "Option 3"),
            ]),
            &Policy::default(),
        );

        // unaffiliated √2 for its two voters. 1: √2/2, 2: √2/2 + √2/2, 3: √2/2 + 1, nobody has
        // half of √2 + √2 + 1
        assert_eq!(tally.rounds[0].eliminated, [1]);
        // duo1 only ranked option 1. Their weight backs no option but still has to be beaten
        let pool = 2f64.sqrt() + 2f64.sqrt() + 1.0;
        assert!(close(
            tally.rounds[1].voted(),
            1.0 - 2f64.sqrt() / 2.0 / pool
        ));
        // 3 leads with √2/2 + 1, which is short of half, and nobody who voted for 2 ranked it
        assert_eq!(tally.rounds.len(), 2);
        assert_eq!(tally.rounds[1].eliminated, [2]);
        assert_eq!(tally.outcome, Outcome::NoMajority);
    }

    #[test]
    fn the_policy_decides_what_the_approved_text_leaves_open() {
        // The whole block behind its only voter
        let whole = Policy {
            unaffiliated_weight: BlockWeight::Roster,
            ..Policy::default()
        };
        let tally = count_with(
            MAIL,
            &votes(&[("duo1", "Option 1"), ("free1", "Option 2")]),
            &whole,
        );
        let pool = 2f64.sqrt() + 3f64.sqrt();
        assert!(close(
            standing(&tally, UNAFFILIATED).weight,
            3f64.sqrt() / pool
        ));
        assert_eq!(tally.outcome, Outcome::Passed(2));

        // duo √2: half abstains, half to 1. unaffiliated 1 abstains
        let abstaining = votes(&[
            ("duo1", "Abstain"),
            ("duo2", "Option 1"),
            ("free1", "Abstain"),
        ]);
        let quorum_only = Policy {
            abstentions: Abstentions::QuorumOnly,
            ..Policy::default()
        };
        let tally = count_with(MAIL, &abstaining, &quorum_only);
        assert!(close(tally.rounds[0].total, score(&tally, 0, 1)));
        assert_eq!(tally.outcome, Outcome::Passed(1));

        // Only duo2 voted, so the block doesn't count
        let ignored = Policy {
            abstentions: Abstentions::Ignored,
            coalition_quorum: 0.25,
            ..Policy::default()
        };
        let tally = count_with(MAIL, &abstaining, &ignored);
        assert_eq!(standing(&tally, "duo").voted, 1);
        assert!(!standing(&tally, UNAFFILIATED).counts);
        assert_eq!(tally.abstained, 0.0);
        assert_eq!(tally.outcome, Outcome::Passed(1));
        assert_eq!(
            count_with(MAIL, &abstaining, &Policy::default()).outcome,
            Outcome::NoMajority
        );

        // duo1 only ranked option 1, which is out, so 3 has more than half of what's left
        let dropped = Policy {
            exhausted: Exhausted::Dropped,
            ..Policy::default()
        };
        let tally = count_with(
            THREE,
            &votes(&[
                ("duo1", "Option 1"),
                ("duo2", "Option 2"),
                ("free1", "Option 3 > Option 1"),
                ("free2", "Option 2"),
                ("solo1", "Option 3"),
            ]),
            &dropped,
        );
        assert!(close(tally.rounds[1].total, tally.rounds[1].voted()));
        assert_eq!(tally.outcome, Outcome::Passed(3));

        // duo √2 to 1, unaffiliated 1 to 3 then 1, solo 1 abstains. 1 only gets past half once
        // it picks up the ballot of 3, the last option it beat
        let two_left = votes(&[
            ("duo1", "Option 1"),
            ("duo2", "Option 1"),
            ("free1", "Option 3 > Option 1"),
            ("solo1", "Abstain"),
        ]);
        let tally = count_with(THREE, &two_left, &Policy::default());
        assert_eq!(tally.rounds.len(), 3);
        assert_eq!(tally.outcome, Outcome::Passed(1));
        let last_two_final = Policy {
            last_two: LastTwo::Final,
            ..Policy::default()
        };
        let tally = count_with(THREE, &two_left, &last_two_final);
        assert_eq!(tally.rounds.len(), 2);
        assert!(tally.rounds[1].eliminated.is_empty());
        assert_eq!(tally.outcome, Outcome::NoMajority);
    }

    #[test]
    fn options_tied_for_last_leave_together() {
        let tally = count_with(
            THREE,
            &votes(&[
                ("duo1", "Option 1 > Option 3"),
                ("duo2", "Option 2 > Option 3"),
                ("solo1", "Option 3"),
            ]),
            &Policy::default(),
        );
        assert_eq!(tally.rounds[0].eliminated, [1, 2]);
        assert_eq!(tally.outcome, Outcome::Passed(3));
    }

    /// Ballots for the rule tests: big 2 for 1 then 2, duo √2 for 2 then 1, solo 1 for 3 only,
    /// and two of the unaffiliated abstain
    fn contested() -> Vec<Reply> {
        votes(&[
            ("big1", "Option 1 > Option 2"),
            ("big2", "Option 1 > Option 2"),
            ("duo1", "Option 2 > Option 1"),
            ("duo2", "Option 2 > Option 1"),
            ("solo1", "Option 3"),
            ("free1", "Abstain"),
            ("free2", "Abstain"),
        ])
    }

    #[test]
    fn every_combination_of_the_rules_counts_as_it_says() {
        use Abstentions as A;
        use Exhausted as E;
        use LastTwo as L;

        let (big, duo, solo) = (2.0, 2f64.sqrt(), 1.0);
        // The unaffiliated weigh √2 for their two voters, or nothing when abstentions are
        // ignored, since then none of them voted
        let pool = |abstentions: A| match abstentions {
            A::Ignored => big + duo + solo,
            A::Counted | A::QuorumOnly => big + duo + solo + 2f64.sqrt(),
        };
        // What option 1 needs more than half of in the last round
        let total = |abstentions: A, exhausted: E, rounds: usize| {
            let abstained = match abstentions {
                A::Counted => 2f64.sqrt(),
                A::QuorumOnly | A::Ignored => 0.0,
            };
            // solo's ballot has no option left from the second round on
            let left = match (exhausted, rounds) {
                (E::Dropped, 2..) => big + duo,
                _ => big + duo + solo,
            };
            abstained + left
        };

        // Option 3 always goes out first. Then option 1 has 2 against √2, which is more than
        // half only when neither the abstentions nor solo's ballot are counted. If the runoff
        // goes on, it also picks up the ballots of option 2
        let passes_in = |rounds: usize| (Outcome::Passed(1), rounds);
        let no_majority = (Outcome::NoMajority, 2);
        let cases = [
            (A::Counted, E::Counted, L::Runoff, passes_in(3)),
            (A::Counted, E::Counted, L::Final, no_majority),
            (A::Counted, E::Dropped, L::Runoff, passes_in(3)),
            (A::Counted, E::Dropped, L::Final, no_majority),
            (A::QuorumOnly, E::Counted, L::Runoff, passes_in(3)),
            (A::QuorumOnly, E::Counted, L::Final, no_majority),
            (A::QuorumOnly, E::Dropped, L::Runoff, passes_in(2)),
            (A::QuorumOnly, E::Dropped, L::Final, passes_in(2)),
            (A::Ignored, E::Counted, L::Runoff, passes_in(3)),
            (A::Ignored, E::Counted, L::Final, no_majority),
            (A::Ignored, E::Dropped, L::Runoff, passes_in(2)),
            (A::Ignored, E::Dropped, L::Final, passes_in(2)),
        ];

        for (abstentions, exhausted, last_two, (outcome, rounds)) in cases {
            let policy = Policy {
                abstentions,
                exhausted,
                last_two,
                ..Policy::default()
            };
            let rules = format!("{abstentions:?}, {exhausted:?}, {last_two:?}");
            let tally = count_with(THREE, &contested(), &policy);
            let pool = pool(abstentions);

            assert_eq!(tally.outcome, outcome, "{rules}");
            assert_eq!(tally.rounds.len(), rounds, "{rules}");
            assert_eq!(tally.rounds[0].eliminated, [3], "{rules}");
            assert_eq!(
                standing(&tally, UNAFFILIATED).counts,
                abstentions != A::Ignored,
                "{rules}"
            );
            // The first round is the same whatever the rules: 2, √2 and 1 of the pool
            assert!(close(score(&tally, 0, 1), big / pool), "{rules}");
            assert!(close(score(&tally, 0, 2), duo / pool), "{rules}");
            assert!(close(score(&tally, 0, 3), solo / pool), "{rules}");

            let last = &tally.rounds[rounds - 1];
            assert!(
                close(last.total, total(abstentions, exhausted, rounds) / pool),
                "{rules}"
            );
            // Option 1 ends with its own weight, plus option 2's after a third round
            let behind = if rounds == 3 { big + duo } else { big };
            assert!(
                close(score(&tally, rounds - 1, 1), behind / pool),
                "{rules}"
            );
            // It passes exactly when that's more than half of the total
            let majority = score(&tally, rounds - 1, 1) > last.total / 2.0;
            assert_eq!(majority, outcome == Outcome::Passed(1), "{rules}");
            if outcome == Outcome::NoMajority {
                // Both options are still in, and nothing moved to option 1
                assert!(last.eliminated.is_empty(), "{rules}");
                assert!(close(score(&tally, 1, 2), duo / pool), "{rules}");
            }
        }
    }

    #[test]
    fn the_last_two_options_tie_whatever_the_rule() {
        // big 2 split between 1 and 2, duo √2 split between 1 and 2, solo 1 for 3 alone
        let replies = votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 2"),
            ("duo1", "Option 1"),
            ("duo2", "Option 2"),
            ("solo1", "Option 3 > Option 1 > Option 2"),
            ("free1", "Option 3 > Option 2 > Option 1"),
        ]);
        for last_two in [LastTwo::Runoff, LastTwo::Final] {
            let policy = Policy {
                last_two,
                ..Policy::default()
            };
            let tally = count_with(THREE, &replies, &policy);
            // 3 leads with 2 of 2 + √2 + 2. 1 and 2 are tied for last and go out together
            assert_eq!(tally.rounds[0].eliminated, [1, 2], "{last_two:?}");
        }

        // Two options with the same weight behind each
        let replies = votes(&[("duo1", "Option 1"), ("duo2", "Option 2")]);
        for last_two in [LastTwo::Runoff, LastTwo::Final] {
            let policy = Policy {
                last_two,
                coalition_quorum: 0.25,
                ..Policy::default()
            };
            let tally = count_with(MAIL, &replies, &policy);
            assert_eq!(tally.outcome, Outcome::Tied, "{last_two:?}");
            assert_eq!(tally.rounds.len(), 1, "{last_two:?}");
        }
    }

    #[test]
    fn the_weight_of_the_unaffiliated_follows_the_rule() {
        // big 2 for 1, one of the three unaffiliated for 2
        let replies = votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 1"),
            ("free1", "Option 2"),
        ]);
        for (unaffiliated_weight, block) in [
            (BlockWeight::Voters, 1.0),
            (BlockWeight::Roster, 3f64.sqrt()),
        ] {
            let policy = Policy {
                unaffiliated_weight,
                ..Policy::default()
            };
            let tally = count_with(MAIL, &replies, &policy);
            let rules = format!("{unaffiliated_weight:?}");
            assert!(
                close(standing(&tally, UNAFFILIATED).weight, block / (2.0 + block)),
                "{rules}"
            );
            assert!(close(score(&tally, 0, 1), 2.0 / (2.0 + block)), "{rules}");
            assert_eq!(tally.outcome, Outcome::Passed(1), "{rules}");
        }
    }

    #[test]
    fn the_quorums_follow_the_rules() {
        // One of big's four members, and both of duo's
        let replies = votes(&[
            ("big1", "Option 1"),
            ("duo1", "Option 2"),
            ("duo2", "Option 2"),
        ]);
        // (internal, coalition, big counts, outcome), with four entities: big, duo, solo and
        // the unaffiliated
        let cases = [
            // big is under half of its roster, and duo alone is only a quarter of the entities
            (0.5, 0.5, false, Outcome::Void),
            (0.5, 0.25, false, Outcome::Passed(2)),
            // A quarter of its roster is enough for big, and 2 beats √2
            (0.25, 0.5, true, Outcome::Passed(1)),
            (0.25, 0.75, true, Outcome::Void),
        ];
        for (internal_quorum, coalition_quorum, counts, outcome) in cases {
            let policy = Policy {
                internal_quorum,
                coalition_quorum,
                ..Policy::default()
            };
            let rules = format!("internal {internal_quorum}, coalition {coalition_quorum}");
            let tally = count_with(MAIL, &replies, &policy);
            assert_eq!(standing(&tally, "big").counts, counts, "{rules}");
            assert_eq!(tally.outcome, outcome, "{rules}");
        }
    }

    #[test]
    fn a_thread_that_isnt_a_vote_is_an_error() {
        let now = sent();
        let counted = tally("hello", sent(), &[], &roster(), &Policy::default(), now);
        assert_eq!(counted.unwrap_err(), MotionError::NoOptions);
    }
}
