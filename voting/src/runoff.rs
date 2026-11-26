//! Instant runoff: eliminate the weakest options until one has a majority

/// Scores closer than this are tied
const EPSILON: f64 = 1e-9;

/// A round of instant runoff
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    /// Share behind each option still in the race
    pub scores: Vec<(u32, f64)>,
    /// Options eliminated after this round
    pub eliminated: Vec<u32>,
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
    NoMajority,
    Tied,
    Void,
    NoVotes,
}

/// Do instant runoff magic. Too much to explain. Refer to governance docs
pub(crate) fn runoff(
    options: &[u32],
    ballots: &[(&[u32], f64)],
    total: f64,
) -> (Vec<Round>, Outcome) {
    let mut running = options.to_vec();
    let mut rounds = Vec::new();

    loop {
        // Each ballot backs its highest ranked option still running
        let scores = running
            .iter()
            .map(|option| {
                let backs = |ranking: &[u32]| {
                    ranking.iter().find(|choice| running.contains(choice)) == Some(option)
                };
                let score = ballots.iter().filter(|(ranking, _)| backs(ranking));
                (*option, score.map(|(_, share)| share).sum())
            })
            .collect();
        let mut round = Round {
            scores,
            eliminated: Vec::new(),
        };

        let winner = round
            .scores
            .iter()
            .find(|(_, s)| *s > total / 2.0 + EPSILON);
        if let Some(&(winner, _)) = winner {
            rounds.push(round);
            return (rounds, Outcome::Passed(winner));
        }
        // The last remaining option only gets a round if it passes
        if round.scores.len() == 1 {
            return (rounds, Outcome::NoMajority);
        }

        // Options tied for last are eliminated together
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

        let outcome = if round.voted() <= EPSILON {
            Some(Outcome::NoVotes)
        } else if eliminated.len() == round.scores.len() {
            Some(Outcome::Tied)
        } else if round.scores.len() == 2 {
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
