//! Counts a vote the way the membership approved it: square-root proportional weighting with
//! a dual-gate quorum, <https://lists.opengamingcollective.org/governance/2ba6fc23-d466-44ec-b472-4d558ec9797d@gmail.com/>
//!
//! - Each organisation weighs the square root of its roster, the unaffiliated members weigh
//!   as one block. Only the members with a valid email are on a roster
//! - Internal quorum: an organisation only counts when enough of its roster voted
//! - Coalition quorum: the vote is void unless enough of the organisations and the block count
//! - The weights are normalised among those that count, each shares its weight equally
//!   between the ballots of its members
//! - More than half of the weight that didn't abstain passes an option, rankings are counted
//!   as an instant runoff
//!
//! What the approved text leaves open is decided here: the quorums come from the [`Policy`],
//! the block of the unaffiliated has no internal quorum since it has no stance to form, and a
//! member listed by several organisations only votes in the first of them

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};

use crate::{
    ballot::{self, Reading, Vote},
    motion::{Motion, MotionError},
    roster::Roster,
};

/// Scores closer than this are tied
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Share of an organisation's roster that has to vote for it to count, from 0 to 1
    pub internal_quorum: f64,
    /// Share of the organisations and the unaffiliated block that have to count, from 0 to 1
    pub coalition_quorum: f64,
    /// How long a vote lasts when its first mail doesn't say
    pub window: TimeDelta,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window: TimeDelta::hours(24),
        }
    }
}

/// A mail of the thread, the first one left out
#[derive(Debug, Clone)]
pub struct Reply {
    /// Name to show
    pub author: String,
    /// Address the mail was really sent from
    pub sender: Option<String>,
    pub date: DateTime<Utc>,
    pub url: String,
    pub body: String,
}

/// The vote of a member, their last one if they voted again
#[derive(Debug, Clone, PartialEq)]
pub struct Ballot {
    /// Username on the roster
    pub member: String,
    pub vote: Vote,
    pub url: String,
    /// Whether any of the member's organisations, or the unaffiliated block, counted
    pub counted: bool,
}

/// A reply that looks like a vote and isn't counted as one
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    pub author: String,
    pub url: String,
    pub reason: Reason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The address isn't the one of a member
    UnknownSender,
    /// Sent before the vote opened or after it closed
    OutsideWindow,
    /// A member who never wrote a vote that could be read
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
    pub voted: usize,
    /// Usernames of its members who voted
    pub voters: Vec<String>,
    /// Whether it passed the internal quorum
    pub counts: bool,
    /// Share of the vote it carries, 0 when it doesn't count
    pub weight: f64,
}

/// A round of the instant runoff, a vote between two options has a single one
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    /// Weight behind each option still in the race, in the order of the motion
    pub scores: Vec<(u32, f64)>,
    /// Out before the next round
    pub eliminated: Vec<u32>,
}

impl Round {
    pub fn total(&self) -> f64 {
        self.scores.iter().map(|(_, score)| score).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed(u32),
    Tied,
    /// The coalition quorum wasn't met
    Void,
    /// The quorum was met and nobody voted for an option
    NoVotes,
}

#[derive(Debug, Clone)]
pub struct Tally {
    pub motion: Motion,
    /// Still open at the time of the count, the outcome can change
    pub open: bool,
    /// Members on the roster, and those of them who can vote
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

/// Count the votes of a thread as of `now`
///
/// `mail` is the body of the first mail, sent at `sent`, `replies` the rest of the thread
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

    let entities = roster.entities();
    let mut standings: Vec<Standing> = entities
        .iter()
        .map(|entity| {
            let roster = entity.members.len();
            let voters: Vec<String> = entity
                .members
                .iter()
                .filter(|member| ballots.contains_key(*member))
                .cloned()
                .collect();
            let voted = voters.len();
            let counts = voted > 0
                && (entity.unaffiliated || voted as f64 >= policy.internal_quorum * roster as f64);
            Standing {
                slug: entity.slug.clone(),
                name: entity.name.clone(),
                unaffiliated: entity.unaffiliated,
                roster,
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
        .map(|standing| (standing.roster as f64).sqrt())
        .sum();
    for standing in standings.iter_mut().filter(|standing| standing.counts) {
        standing.weight = (standing.roster as f64).sqrt() / pool;
    }

    let counting = standings.iter().filter(|standing| standing.counts).count();
    let void =
        counting == 0 || (counting as f64) < policy.coalition_quorum * standings.len() as f64;

    // Each ballot carries its share of the weight of each entity its member votes in
    let mut shares: BTreeMap<&str, f64> = BTreeMap::new();
    for (entity, standing) in entities.iter().zip(&standings) {
        if !standing.counts {
            continue;
        }
        for member in &entity.members {
            if ballots.contains_key(member) {
                *shares.entry(member.as_str()).or_default() +=
                    standing.weight / standing.voted as f64;
            }
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
        runoff(&options, &ranked)
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

/// The last readable vote of each member, and the replies that should have been one
fn collect(
    motion: &Motion,
    replies: &[Reply],
    roster: &Roster,
) -> (BTreeMap<String, Ballot>, Vec<Rejected>) {
    let mut replies: Vec<&Reply> = replies.iter().collect();
    replies.sort_by_key(|reply| reply.date);

    let mut ballots: BTreeMap<String, Ballot> = BTreeMap::new();
    let mut rejected = Vec::new();
    // Members who wrote without a vote, they only matter if they never vote
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
            // Anyone can comment at any time, only a clear vote is worth telling. The count
            // posted once the vote is closed lists the options too
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

/// Drop the weakest options until one holds more than half of the weight still in the race
fn runoff(options: &[u32], ballots: &[(&[u32], f64)]) -> (Vec<Round>, Outcome) {
    let mut running: Vec<u32> = options.to_vec();
    let mut rounds = Vec::new();

    loop {
        let mut scores: Vec<(u32, f64)> = running.iter().map(|option| (*option, 0.0)).collect();
        for (ranking, share) in ballots {
            // Ballots that only ranked eliminated options leave the race
            let Some(choice) = ranking.iter().find(|option| running.contains(option)) else {
                continue;
            };
            if let Some(score) = scores.iter_mut().find(|score| score.0 == *choice) {
                score.1 += share;
            }
        }

        let total: f64 = scores.iter().map(|(_, score)| score).sum();
        let lowest = scores
            .iter()
            .map(|(_, score)| *score)
            .fold(f64::MAX, f64::min);
        let winner = scores
            .iter()
            .find(|(_, score)| *score > total / 2.0 + EPSILON)
            .map(|(option, _)| *option);
        // Options tied for the last place leave together
        let eliminated: Vec<u32> = scores
            .iter()
            .filter(|(_, score)| winner.is_none() && *score <= lowest + EPSILON)
            .map(|(option, _)| *option)
            .collect();

        let stuck = eliminated.len() == scores.len();
        rounds.push(Round {
            scores,
            eliminated: if stuck {
                Vec::new()
            } else {
                eliminated.clone()
            },
        });

        if let Some(winner) = winner {
            return (rounds, Outcome::Passed(winner));
        }
        if total <= EPSILON {
            return (rounds, Outcome::NoVotes);
        }
        if stuck {
            return (rounds, Outcome::Tied);
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

    /// `big` has four members, `duo` two, `solo` one, and four members are unaffiliated, `mute`
    /// has no email
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
        score / round.total()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Everyone votes for the option after their name
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

        // big 2, duo √2, unaffiliated √3. solo also lists big1, who only votes in big
        let pool = 2.0 + 2f64.sqrt() + 3f64.sqrt();
        assert!(close(standing(&tally, "big").weight, 2.0 / pool));
        assert!(close(standing(&tally, "duo").weight, 2f64.sqrt() / pool));
        let solo = standing(&tally, "solo");
        assert_eq!((solo.roster, solo.voted, solo.counts), (1, 0, false));
        assert!(close(
            standing(&tally, UNAFFILIATED).weight,
            3f64.sqrt() / pool
        ));
        // mute can't vote, the block is the size of the three who can
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
        // No quorum for the unaffiliated
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
    fn abstentions_count_for_the_quorum_only() {
        let tally = count(&votes(&[
            ("duo1", "Abstain"),
            ("duo2", "Option 1"),
            ("free1", "abstain"),
        ]));
        assert!(standing(&tally, "duo").counts);
        assert_eq!(tally.outcome, Outcome::Passed(1));
        assert!(close(score(&tally, 0, 1), 1.0));

        let pool = 2f64.sqrt() + 3f64.sqrt();
        assert!(close(
            tally.abstained,
            (2f64.sqrt() / 2.0 + 3f64.sqrt()) / pool
        ));

        let tally = count(&votes(&[("duo1", "Abstain"), ("free1", "Abstain")]));
        assert_eq!(tally.outcome, Outcome::NoVotes);
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

        // duo √2: half to 1, half to 3. unaffiliated √3 to 2. solo 1 to 3
        assert_eq!(tally.rounds.len(), 2);
        assert_eq!(tally.rounds[0].eliminated, [1]);
        assert_eq!(tally.rounds[1].scores.len(), 2);
        // duo1's ballot moves to option 2
        let pool = 2f64.sqrt() + 3f64.sqrt() + 1.0;
        assert!(close(
            score(&tally, 1, 2),
            (3f64.sqrt() + 2f64.sqrt() / 2.0) / pool
        ));
        assert_eq!(tally.outcome, Outcome::Passed(2));
    }

    #[test]
    fn ballots_out_of_options_leave_the_runoff() {
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

        // 1: √2/2, 2: √2/2 + √3/2, 3: √3/2 + 1, nobody has half of √2 + √3 + 1
        assert_eq!(tally.rounds[0].eliminated, [1]);
        // duo1 only ranked option 1, the race goes on without them
        let pool = 2f64.sqrt() + 3f64.sqrt() + 1.0;
        assert!(close(
            tally.rounds[1].total(),
            1.0 - 2f64.sqrt() / 2.0 / pool
        ));
        assert_eq!(tally.outcome, Outcome::Passed(3));
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

    #[test]
    fn a_thread_that_isnt_a_vote_is_an_error() {
        let now = sent();
        let counted = tally("hello", sent(), &[], &roster(), &Policy::default(), now);
        assert_eq!(counted.unwrap_err(), MotionError::NoOptions);
    }
}
