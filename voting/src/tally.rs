//! Count and tally votes according to OGC governance approved voting method:
//!
//! Square-Root Proportional Weighting and Dual-Gate Quorum Model

use jiff::{SignedDuration, Timestamp};

use crate::{
    ballot::{Ballot, Rejected, Reply, Vote, collect},
    motion::{Motion, MotionError},
    policy::Policy,
    roster::{Entity, Roster},
    runoff::{Outcome, Round, runoff},
};

/// An organisation or the unaffiliated block, and how it voted
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub entity: Entity,
    pub ballots: Vec<Ballot>,
    pub passed_quorum: bool,
    pub weight: f64,
}

#[derive(Debug, Clone)]
pub struct Tally {
    pub motion: Motion,
    pub open: bool,
    pub rejected: Vec<Rejected>,
    pub standings: Vec<Standing>,
    pub needed: usize,
    pub rounds: Vec<Round>,
    pub total: f64,
    pub outcome: Outcome,
}

/// Count the votes on a thread as of `now`
pub fn tally(
    mail: &str,
    sent: Timestamp,
    replies: &[Reply],
    roster: &Roster,
    policy: &Policy,
    now: Timestamp,
) -> Result<Tally, MotionError> {
    let window = SignedDuration::from_hours(policy.window_hours.into());
    let motion = Motion::read(mail, sent, window)?;
    let (mut member_ballots, rejected) = collect(&motion, replies, roster);

    let mut standings: Vec<Standing> = roster
        .entities()
        .into_iter()
        .map(|entity| {
            let ballots: Vec<Ballot> = entity
                .members
                .iter()
                .filter_map(|member| member_ballots.remove(member))
                .collect();
            let quorum = policy.internal_quorum * entity.members.len() as f64;
            let passed_quorum =
                !ballots.is_empty() && (entity.unaffiliated || (ballots.len() as f64 >= quorum));
            Standing {
                entity,
                ballots,
                passed_quorum,
                weight: 0.0,
            }
        })
        .collect();

    // An organisation weighs its whole roster, the unaffiliated block only those who voted
    let root = |standing: &Standing| {
        let size = if standing.entity.unaffiliated {
            standing.ballots.len()
        } else {
            standing.entity.members.len()
        };
        (size as f64).sqrt()
    };

    // total voting pool. each org passing quorum gets sqrt of member count calculated and summed for the pool
    // unaffiliated block gets users who voted as member count
    let pool: f64 = standings.iter().filter(|s| s.passed_quorum).map(root).sum();

    // each org gets a percentage (0-1 scale) of the total pool assigned to it based on the square root weight
    for standing in standings.iter_mut().filter(|s| s.passed_quorum) {
        standing.weight = root(standing) / pool;
    }

    let counting = standings.iter().filter(|s| s.passed_quorum).count();
    let needed = (policy.coalition_quorum * standings.len() as f64).ceil() as usize;
    let void = counting == 0 || counting < needed;

    // Each voter gets an equal part of their standing's weight.
    // Abstentions get dropped here. They are unranked but helped reach quorum
    let ranked: Vec<(&[u32], f64)> = standings
        .iter()
        .filter(|s| s.passed_quorum)
        .flat_map(|s| {
            let share = s.weight / s.ballots.len() as f64;
            s.ballots
                .iter()
                .filter_map(move |ballot| match &ballot.vote {
                    Vote::Ranked(options) => Some((options.as_slice(), share)),
                    Vote::Abstain => None,
                })
        })
        .collect();

    // We have to do this part because with abstentions dropped, total may no longer be 1
    let total: f64 = ranked.iter().map(|(_, share)| share).sum();

    let options: Vec<u32> = motion.options.iter().map(|option| option.number).collect();
    let (rounds, outcome) = if void {
        (Vec::new(), Outcome::Void)
    } else {
        runoff(&options, &ranked, total)
    };

    Ok(Tally {
        open: now < motion.closes,
        motion,
        rejected,
        standings,
        needed,
        rounds,
        total,
        outcome,
    })
}

impl Tally {
    /// Number of entities that passed quorum
    pub fn counting(&self) -> usize {
        self.standings.iter().filter(|s| s.passed_quorum).count()
    }

    /// Total ballots cast
    pub fn cast(&self) -> usize {
        self.standings.iter().map(|s| s.ballots.len()).sum()
    }

    /// Share of the counted weight
    pub fn share(&self, weight: f64) -> f64 {
        if self.total > 0.0 {
            weight / self.total
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ballot::Reason, motion::tests::sent};

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

    fn standing<'a>(tally: &'a Tally, name: &str) -> &'a Standing {
        tally
            .standings
            .iter()
            .find(|standing| standing.entity.name == name)
            .unwrap()
    }

    fn ballots(tally: &Tally) -> Vec<&Ballot> {
        tally
            .standings
            .iter()
            .flat_map(|standing| &standing.ballots)
            .collect()
    }

    fn unaffiliated(tally: &Tally) -> &Standing {
        tally
            .standings
            .iter()
            .find(|standing| standing.entity.unaffiliated)
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
        assert_eq!(
            (
                solo.entity.members.len(),
                solo.ballots.len(),
                solo.passed_quorum
            ),
            (1, 0, false)
        );
        assert!(close(unaffiliated(&tally).weight, 1.0 / pool));
        // mute can't vote, so the block only counts the three who can
        assert_eq!(unaffiliated(&tally).entity.members.len(), 3);

        assert!(close(score(&tally, 0, 1), 2.0 / pool));
        assert_eq!(tally.outcome, Outcome::Passed(2));
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
        assert_eq!(
            (
                big.ballots.len(),
                big.entity.members.len(),
                big.passed_quorum
            ),
            (1, 4, false)
        );
        assert_eq!(big.weight, 0.0);
        assert!(standing(&tally, "duo").passed_quorum);
        // The unaffiliated block has no quorum
        assert!(unaffiliated(&tally).passed_quorum);

        assert!(close(score(&tally, 0, 2), 1.0));
        assert_eq!(tally.outcome, Outcome::Passed(2));
        assert!(big.ballots.iter().any(|ballot| ballot.member == "big2"));
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
        assert!(standing(&tally, "duo").passed_quorum);
        let pool = 2f64.sqrt() + 1.0;
        assert!(close(score(&tally, 0, 1), 2f64.sqrt() / 2.0 / pool));
        assert!(close(tally.total, score(&tally, 0, 1)));
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
        assert_eq!(tally.outcome, Outcome::NoMajority);
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
            let ballot = ballots(&tally)
                .into_iter()
                .find(|ballot| ballot.member == member);
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
        assert!(ballots(&tally).is_empty());
        assert_eq!(tally.rejected[1].url, "https://l/mallory/1/");
    }

    #[test]
    fn the_count_posted_after_the_vote_is_not_a_late_vote() {
        let results =
            "Results after 24h\n\nOption 1: 2/2 votes (100 %)\nOption 2: 0/2 votes (0%)\n";
        let tally = count(&[reply("duo1", 1, "Option 1"), reply("duo1", 26, results)]);
        assert!(tally.rejected.is_empty());
        assert_eq!(ballots(&tally).len(), 1);
    }

    #[test]
    fn the_vote_closes_when_the_mail_says() {
        let on_time = [reply("duo1", 24, "Option 1"), reply("free1", 0, "Option 1")];
        assert_eq!(ballots(&count(&on_time)).len(), 2);

        let now = sent() + SignedDuration::from_hours(23);
        let open = tally(MAIL, sent(), &[], &roster(), &Policy::default(), now).unwrap();
        assert!(open.open);
        assert!(!count(&[]).open);

        // The mail doesn't say
        let long = Policy {
            window_hours: 72,
            ..Policy::default()
        };
        let late = [reply("duo1", 48, "Option 1 > Option 2")];
        assert_eq!(ballots(&count_with(THREE, &late, &long)).len(), 1);
        assert!(ballots(&count_with(THREE, &late, &Policy::default())).is_empty());
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
            tally.rounds[1].continuing_weight(),
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
        assert!(close(unaffiliated(&tally).weight, 1.0 / 3.0));
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
            assert_eq!(standing(&tally, "big").passed_quorum, counts, "{rules}");
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
