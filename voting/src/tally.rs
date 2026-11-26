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
//! Things the approved text leaves open:
//!
//! - The unaffiliated block has no internal quorum
//! - A member of several organisations votes in the first one by slug
//! - Options tied for last are eliminated together
//! - The unaffiliated block weighs as many as voted
//! - Abstentions count toward the quorums, not the majority
//! - A ballot with no option left still counts toward the majority
//! - Two options left without a majority get no runoff

use std::collections::BTreeMap;

use crate::{
    ballot::{self, Vote},
    motion::{Motion, MotionError},
    roster::Roster,
};
use jiff::{SignedDuration, Timestamp};

/// Scores closer than this are tied
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Fraction of an organisation's members who must vote for it to count (0 to 1)
    pub internal_quorum: f64,
    /// Fraction of the entities that must count for the vote to be valid (0 to 1)
    pub coalition_quorum: f64,
    /// How long a vote stays open if its first mail doesn't say
    pub window: SignedDuration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window: SignedDuration::from_hours(24),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    /// Display name
    pub author: String,
    /// Sender address
    pub sender: Option<String>,
    pub date: Timestamp,
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
    UnknownSender,
    /// Sent before the vote opened or after it closed
    OutsideWindow,
    /// A member who never wrote a valid vote
    Invalid,
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
    pub voters: Vec<String>,
    /// Whether it passed the internal quorum
    pub counts: bool,
    /// Its share of the vote, or 0 if it doesn't count
    pub weight: f64,
}

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
    sent: Timestamp,
    replies: &[Reply],
    roster: &Roster,
    policy: &Policy,
    now: Timestamp,
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
                size: if entity.unaffiliated { voted } else { roster },
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

/// Each member's last valid vote, and the rejected replies
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
    let mut invalid: BTreeMap<String, &Reply> = BTreeMap::new();

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
            // Anyone can comment. Only a valid vote is worth reporting
            if reading.is_some() {
                rejected.push(reject(match member {
                    Some(_) => Reason::OutsideWindow,
                    None => Reason::UnknownSender,
                }));
            }
            continue;
        };

        match reading {
            Some(vote) => {
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
            None => {
                invalid.entry(member.to_string()).or_insert(reply);
            }
        }
    }

    for (member, reply) in invalid {
        if !ballots.contains_key(&member) {
            rejected.push(Rejected {
                author: reply.author.clone(),
                url: reply.url.clone(),
                reason: Reason::Invalid,
            });
        }
    }
    (ballots, rejected)
}

/// Eliminate the weakest options until one has more than half of the counted weight,
/// including the ballots with no option left
fn runoff(options: &[u32], ballots: &[(&[u32], f64)]) -> (Vec<Round>, Outcome) {
    let mut running: Vec<u32> = options.to_vec();
    let mut rounds = Vec::new();
    let total: f64 = ballots.iter().map(|(_, share)| share).sum();

    loop {
        let mut scores: Vec<(u32, f64)> = running.iter().map(|option| (*option, 0.0)).collect();
        for (ranking, share) in ballots {
            let Some(choice) = ranking.iter().find(|option| running.contains(option)) else {
                continue;
            };
            if let Some(score) = scores.iter_mut().find(|score| score.0 == *choice) {
                score.1 += share;
            }
        }

        let voted: f64 = scores.iter().map(|(_, score)| score).sum();
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
        let last = winner.is_none() && scores.len() <= 2;
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
            date: sent() + SignedDuration::from_hours(hours),
            url: format!("https://l/{member}/{hours}/"),
            body: format!("VOTE BELOW THIS LINE\n{body}"),
        }
    }

    fn count_with(mail: &str, replies: &[Reply], policy: &Policy) -> Tally {
        let now = sent() + SignedDuration::from_hours(7 * 24);
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
    fn abstentions_count_toward_the_quorums_not_the_majority() {
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
        assert!(close(tally.rounds[0].total, score(&tally, 0, 1)));
        assert_eq!(tally.outcome, Outcome::Passed(1));

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
    fn the_last_vote_of_a_member_counts() {
        let tally = count(&[
            reply("duo1", 1, "Option 1"),
            reply("free1", 3, "Option 2"),
            reply("free1", 2, "Option 1"),
            reply("duo1", 4, "Actually, let me explain why."),
            reply("duo1", 5, "Option 9"),
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
                ("duo1", Reason::OutsideWindow),
                ("big2", Reason::Invalid),
                ("big3", Reason::Invalid),
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

        let now = sent() + SignedDuration::from_hours(23);
        let open = tally(MAIL, sent(), &[], &roster(), &Policy::default(), now).unwrap();
        assert!(open.open);
        assert!(!count(&[]).open);

        // The mail doesn't say
        let long = Policy {
            window: SignedDuration::from_hours(72),
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
        // 3 leads with √2/2 + 1, which is short of half
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

    #[test]
    fn the_last_two_options_tie() {
        // big 2 split between 1 and 2, duo √2 split between 1 and 2, solo 1 for 3 alone
        let replies = votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 2"),
            ("duo1", "Option 1"),
            ("duo2", "Option 2"),
            ("solo1", "Option 3 > Option 1 > Option 2"),
            ("free1", "Option 3 > Option 2 > Option 1"),
        ]);
        let tally = count_with(THREE, &replies, &Policy::default());
        // 3 leads with 2 of 2 + √2 + 2. 1 and 2 are tied for last and go out together
        assert_eq!(tally.rounds[0].eliminated, [1, 2]);

        // Two options with the same weight behind each
        let replies = votes(&[("duo1", "Option 1"), ("duo2", "Option 2")]);
        let policy = Policy {
            coalition_quorum: 0.25,
            ..Policy::default()
        };
        let tally = count_with(MAIL, &replies, &policy);
        assert_eq!(tally.outcome, Outcome::Tied);
        assert_eq!(tally.rounds.len(), 1);
    }

    #[test]
    fn the_unaffiliated_weigh_as_many_as_voted() {
        // big 2 for 1, one of the three unaffiliated for 2
        let replies = votes(&[
            ("big1", "Option 1"),
            ("big2", "Option 1"),
            ("free1", "Option 2"),
        ]);
        let tally = count(&replies);
        assert!(close(standing(&tally, UNAFFILIATED).weight, 1.0 / 3.0));
        assert!(close(score(&tally, 0, 1), 2.0 / 3.0));
        assert_eq!(tally.outcome, Outcome::Passed(1));
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
