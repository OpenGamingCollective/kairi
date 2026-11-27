//! Instant runoff: eliminate the weakest options until one has a majority

/// Scores closer than this are tied
const EPSILON: f64 = 1e-9;

/// A round of instant runoff
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    pub scores: Vec<(u32, f64)>,
    pub eliminated: Vec<u32>,
    pub choices: Vec<(String, Option<u32>)>,
}

impl Round {
    /// Total weight continuing after this round
    pub fn continuing_weight(&self) -> f64 {
        self.scores.iter().map(|(_, score)| score).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed(u32),
    /// No option got more than half of the weight after runoff
    NoMajority,
    /// All remaining options tied
    Tied,
    /// Did not reach coalition quorum
    Void,
}

/// Do instant runoff magic. Too much to explain. Refer to governance docs
pub(crate) fn runoff(
    options: &[u32],
    ballots: &[(&str, &[u32], f64)],
    total: f64,
) -> (Vec<Round>, Outcome) {
    let mut running = options.to_vec();
    let mut rounds = Vec::new();

    loop {
        let choices: Vec<(String, Option<u32>)> = ballots
            .iter()
            .map(|(member, ranking, _)| {
                // from the member's choices, the first choice that is still running
                let choice = ranking.iter().copied().find(|c| running.contains(c));
                (member.to_string(), choice)
            })
            .collect();
        let scores = running
            .iter()
            .map(|option| {
                let backing = ballots.iter().zip(&choices);
                let score = backing.filter(|(_, (_, choice))| *choice == Some(*option));
                (*option, score.map(|((_, _, share), _)| share).sum())
            })
            .collect();
        let mut round = Round {
            scores,
            eliminated: Vec::new(),
            choices,
        };

        let winner = round
            .scores
            .iter()
            .find(|(_, s)| *s > total / 2.0 + EPSILON);
        if let Some(&(winner, _)) = winner {
            rounds.push(round);
            return (rounds, Outcome::Passed(winner));
        }

        // options tied for last are eliminated together
        let lowest = round
            .scores
            .iter()
            .map(|(_, s)| *s)
            .fold(f64::MAX, f64::min);
        let eliminated: Vec<u32> = round
            .scores
            .iter()
            .filter(|(_, score)| *score <= lowest + EPSILON)
            .map(|(option, _)| *option)
            .collect();

        let options_left = round.scores.len();
        let all_tied_for_last = eliminated.len() == options_left;
        let has_continuing_ballots = round.continuing_weight() > EPSILON;

        let outcome = if all_tied_for_last && has_continuing_ballots && options_left > 1 {
            Some(Outcome::Tied)
        } else if !has_continuing_ballots || options_left <= 2 {
            Some(Outcome::NoMajority)
        } else {
            None
        };
        if outcome.is_none() {
            running.retain(|option| !eliminated.contains(option));
            round.eliminated = eliminated;
        }
        rounds.push(round);
        if let Some(outcome) = outcome {
            return (rounds, outcome);
        }
    }
}
