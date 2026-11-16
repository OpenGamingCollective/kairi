//! Reads the vote a member wrote in their reply
//!
//! Only what the member wrote counts, never what their mail client quoted. A vote is a line
//! that is nothing but a vote, `Option 1`, `I vote for option 2`, `2 > 1 > 3` or `Abstain`,
//! wherever it is in the reply

use crate::motion::{Motion, fold};

/// Words allowed in front of the option, as in `I vote for option 1`
const FILLER: &[&str] = &[
    "+1", "a", "am", "an", "ballot", "be", "cast", "choice", "choose", "favor", "favour", "for",
    "go", "going", "i", "i'd", "i'll", "i'm", "in", "is", "it's", "like", "me", "my", "of", "pick",
    "prefer", "select", "support", "the", "to", "vote", "voted", "votes", "voting", "we", "will",
    "with", "would",
];

/// Words allowed after the option, as in `Option 1 because ...`
const REASON: &[&str] = &["as", "because", "for", "please", "since", "thanks"];

const NUMBERS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vote {
    Abstain,
    /// Option numbers, the preferred one first
    Ranked(Vec<u32>),
}

/// What a reply says
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Vote(Vote),
    /// A comment, or a vote written in a way that isn't understood
    NoVote,
    /// Several votes that don't agree
    Conflicting,
    /// A vote for an option the motion doesn't have
    UnknownOption(u32),
}

/// A line that is a vote
#[derive(Debug, PartialEq, Eq)]
enum Line {
    Abstain,
    /// `rank` is the `2.` of `2. Option 1`
    Options {
        rank: Option<u32>,
        options: Vec<u32>,
    },
    UnknownOption(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Word(&'a str),
    Punctuation(char),
}

/// Read the body of a reply to `motion`
pub fn read(body: &str, motion: &Motion) -> Reading {
    let lines: Vec<Line> = written(body, motion)
        .into_iter()
        .filter_map(|line| read_line(line, motion))
        .collect();

    if let Some(unknown) = lines.iter().find_map(|line| match line {
        Line::UnknownOption(number) => Some(*number),
        _ => None,
    }) {
        return Reading::UnknownOption(unknown);
    }

    let abstains = lines.contains(&Line::Abstain);
    let mut votes: Vec<(Option<u32>, &[u32])> = lines
        .iter()
        .filter_map(|line| match line {
            Line::Options { rank, options } => Some((*rank, options.as_slice())),
            _ => None,
        })
        .collect();
    // The same vote at the top and at the bottom is one vote, numbered lines get sorted by rank
    votes.sort_unstable();
    votes.dedup();
    let unique = votes;

    match (abstains, unique.len()) {
        (false, 0) => Reading::NoVote,
        (true, 0) => Reading::Vote(Vote::Abstain),
        (true, _) => Reading::Conflicting,
        (false, 1) => Reading::Vote(Vote::Ranked(unique[0].1.to_vec())),
        // One option per line, only a ranking when each line says its rank
        (false, _) => {
            let numbered = unique
                .iter()
                .all(|(rank, options)| rank.is_some() && options.len() == 1);
            let options: Vec<u32> = unique.iter().map(|(_, options)| options[0]).collect();
            let ranks: Vec<Option<u32>> = unique.iter().map(|(rank, _)| *rank).collect();
            if numbered && !repeats(&options) && !repeats(&ranks) {
                Reading::Vote(Vote::Ranked(options))
            } else {
                Reading::Conflicting
            }
        }
    }
}

fn repeats<T: PartialEq>(items: &[T]) -> bool {
    items
        .iter()
        .enumerate()
        .any(|(position, item)| items[..position].contains(item))
}

/// Lines the member wrote, without the mail they reply to
fn written<'a>(body: &'a str, motion: &Motion) -> Vec<&'a str> {
    let unquoted: Vec<&str> = body
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect();

    // The cut line is only left unquoted by clients that don't prefix what they quote, the
    // first mail is then everything from its first line down to the cut line
    let cut = motion.cut_line.as_ref().and_then(|cut_line| {
        unquoted
            .iter()
            .rposition(|line| fold(line).contains(cut_line))
    });
    let Some(cut) = cut else {
        return unquoted;
    };

    let quote = unquoted[..cut]
        .iter()
        .position(|line| quote_header(line) || motion.quotes(line))
        .unwrap_or(cut);
    let mut written = unquoted[..quote].to_vec();
    written.extend(&unquoted[cut + 1..]);
    written
}

/// Line a mail client writes above the mail it quotes
fn quote_header(line: &str) -> bool {
    let line = fold(line);
    line.contains("original message")
        || line.contains("forwarded message")
        || line.starts_with("from:")
        || (line.starts_with("on ") && line.ends_with("wrote:"))
}

fn read_line(line: &str, motion: &Motion) -> Option<Line> {
    let line = fold(line).replace('’', "'");
    let tokens = tokens(&line);
    let mut rest = tokens.as_slice();

    let words = || {
        tokens.iter().filter_map(|token| match token {
            Token::Word(word) => Some(*word),
            Token::Punctuation(_) => None,
        })
    };
    if words().any(|word| word.starts_with("abstain") || word == "abstention") {
        let only_abstains = words().all(|word| {
            word.starts_with("abstain") || word == "abstention" || FILLER.contains(&word)
        });
        return only_abstains.then_some(Line::Abstain);
    }

    // `1. Option 2`, the first number is a rank
    let mut rank = None;
    if let [
        Token::Word(number),
        Token::Punctuation('.' | ')'),
        Token::Word(word),
        ..,
    ] = rest
        && option_word(word)
    {
        rank = Some(number.parse().ok()?);
        rest = &rest[2..];
    }

    // Decoration and filler
    while let [first, tail @ ..] = rest {
        let skipped = match first {
            Token::Word(word) => FILLER.contains(word),
            Token::Punctuation(_) => true,
        };
        if !skipped {
            break;
        }
        rest = tail;
    }

    let mut options = Vec::new();
    let mut explicit = false;
    loop {
        let (number, named, tail) = reference(rest)?;
        explicit |= named;
        if motion.option(number).is_none() {
            // A lone number can be anything
            return named.then_some(Line::UnknownOption(number));
        }
        if options.contains(&number) {
            return None;
        }
        options.push(number);
        rest = tail;

        // `Option 1 > Option 2`
        match separator(rest).filter(|tail| reference(tail).is_some()) {
            Some(tail) => rest = tail,
            None => break,
        }
    }

    let trailing = match rest {
        [] => true,
        // `Option 1: its title`, not `Option 1/2`
        [Token::Punctuation(mark), ..] if explicit => *mark != '/',
        [Token::Word(word), ..] if explicit => REASON.contains(word),
        // `1. I think that...` is not a vote
        rest => rest
            .iter()
            .all(|token| matches!(token, Token::Punctuation(_))),
    };
    trailing.then_some(Line::Options { rank, options })
}

/// `option 1`, `option1`, `#1` or `1`, with whether it names itself an option
fn reference<'t, 'a>(tokens: &'t [Token<'a>]) -> Option<(u32, bool, &'t [Token<'a>])> {
    match tokens {
        [Token::Word(word), tail @ ..] if option_word(word) => {
            let tail = match tail {
                [Token::Punctuation('#'), tail @ ..] => tail,
                tail => tail,
            };
            match tail {
                [Token::Word(number), tail @ ..] => Some((number_of(number)?, true, tail)),
                _ => None,
            }
        }
        [Token::Punctuation('#'), Token::Word(number), tail @ ..] => {
            Some((number.parse().ok()?, false, tail))
        }
        [Token::Word(word), tail @ ..] => match word.strip_prefix("option") {
            Some(number) => Some((number.parse().ok()?, true, tail)),
            None => Some((word.parse().ok()?, false, tail)),
        },
        _ => None,
    }
}

fn option_word(word: &str) -> bool {
    matches!(word, "option" | "options" | "opt")
}

fn number_of(word: &str) -> Option<u32> {
    word.parse().ok().or_else(|| {
        let position = NUMBERS.iter().position(|number| *number == word)?;
        u32::try_from(position + 1).ok()
    })
}

/// What ranks an option above the next one
fn separator<'t, 'a>(tokens: &'t [Token<'a>]) -> Option<&'t [Token<'a>]> {
    match tokens {
        [
            Token::Punctuation('-' | '='),
            Token::Punctuation('>'),
            tail @ ..,
        ] => Some(tail),
        [Token::Punctuation('>' | ',' | ';' | '→'), tail @ ..] => Some(tail),
        [Token::Word("then"), tail @ ..] => Some(tail),
        _ => None,
    }
}

/// Words and the punctuation between them, `don't` and `+1` are words
fn tokens(line: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut word = None;
    for (position, character) in line.char_indices() {
        if character.is_alphanumeric() || matches!(character, '\'' | '+') {
            word.get_or_insert(position);
            continue;
        }
        if let Some(start) = word.take() {
            tokens.push(Token::Word(&line[start..position]));
        }
        if !character.is_whitespace() {
            tokens.push(Token::Punctuation(character));
        }
    }
    if let Some(start) = word {
        tokens.push(Token::Word(&line[start..]));
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::tests::{MAIL, motion};

    fn quoted(mail: &str) -> String {
        mail.lines().map(|line| format!("> {line}\n")).collect()
    }

    fn vote(body: &str) -> Reading {
        read(body, &motion(MAIL))
    }

    fn ranked(options: &[u32]) -> Reading {
        Reading::Vote(Vote::Ranked(options.to_vec()))
    }

    /// Three options, no cut line
    fn vote_of_three(body: &str) -> Reading {
        read(body, &motion("Option 1: a\nOption 2: b\nOption 3: c\n"))
    }

    #[test]
    fn vote_under_the_quoted_mail() {
        let body = format!(
            "On Fri, Sep 11, 2026 at 4:47 PM Derek J. Clark\n<derek@example.org> wrote:\n{}>\n\nOption 1\n\n",
            quoted(MAIL)
        );
        assert_eq!(vote(&body), ranked(&[1]));
    }

    #[test]
    fn vote_above_the_quoted_mail() {
        let body = format!(
            "Option 2\n\nOn 9/12/26 01:47, Derek wrote:\n{}",
            quoted(MAIL)
        );
        assert_eq!(vote(&body), ranked(&[2]));
    }

    #[test]
    fn vote_without_the_quoted_mail() {
        assert_eq!(vote("Option 1"), ranked(&[1]));
        assert_eq!(vote("Hi all,\n\noption 2.\n\n-- \nAlice\n"), ranked(&[2]));
    }

    #[test]
    fn same_vote_above_and_under_is_one_vote() {
        let body = format!(
            "Option 1\n\n-------- Original Message --------\n{}\nOption 1\n",
            quoted(MAIL)
        );
        assert_eq!(vote(&body), ranked(&[1]));
    }

    #[test]
    fn quote_prefixed_with_spaces() {
        let quoted: String = MAIL.lines().map(|line| format!(" > {line}\n")).collect();
        assert_eq!(vote(&format!("{quoted}\nOption 1\n")), ranked(&[1]));
    }

    #[test]
    fn quoted_votes_are_not_counted() {
        // The reply to another member's vote
        let body = format!("> Option 1\n{}\nI disagree with this.\n", quoted(MAIL));
        assert_eq!(vote(&body), Reading::NoVote);
        assert_eq!(vote(&quoted(MAIL)), Reading::NoVote);
    }

    #[test]
    fn mail_quoted_without_a_prefix() {
        let under = format!("From: Derek\nSent: Friday\n\n{MAIL}\nOption 2\n");
        assert_eq!(vote(&under), ranked(&[2]));

        let above = format!("Option 2\n\nOn Friday, Derek wrote:\n\n{MAIL}");
        assert_eq!(vote(&above), ranked(&[2]));

        // No header at all, the mail's own lines give it away
        let bare = format!("option 1\n\n{MAIL}");
        assert_eq!(vote(&bare), ranked(&[1]));

        assert_eq!(vote(MAIL), Reading::NoVote);
    }

    #[test]
    fn ways_to_write_a_vote() {
        for body in [
            "Option 1",
            "option 1",
            "OPTION 1.",
            "Option 1!",
            "Option #1",
            "Option1",
            "opt 1",
            "option one",
            "1",
            "#1",
            "1.",
            "*Option 1*",
            "**Option 1**",
            "- Option 1",
            "\u{a0}Option\u{a0}1 ",
            "I vote for Option 1",
            "I vote Option 1",
            "I’m voting for option 1",
            "I'm voting for option 1",
            "My vote: Option 1",
            "My vote is for Option 1.",
            "Vote: 1",
            "+1 for Option 1",
            "I'd like to go with option 1",
            "Option 1: Square-Root Proportional Weighting and Dual-Gate Quorum Model",
            "Option 1 - the square root one",
            "Option 1 (reluctantly)",
            "Option 1, thanks for organising",
            "Option 1 because it scales",
        ] {
            assert_eq!(vote(body), ranked(&[1]), "{body}");
        }
    }

    #[test]
    fn comments_are_not_votes() {
        for body in [
            "",
            "Thanks for organising this!",
            "I think option 1 has flaws",
            "Option 1 is a bad idea",
            "Not option 2",
            "I will not vote for option 1",
            "Is it either Option 1 or Option 2?",
            "Option 1 or Option 2",
            "Option 1/2",
            "Options",
            "1. I disagree with the premise",
            "2 cents from me",
            "12 Sept 2026 05:17:44 Derek J. Clark <derek@example.org>:",
            "On Friday, September 11, 2026 23:47 UTC, Derek wrote:",
            "2026",
            "+1",
            "Sent from my phone",
        ] {
            assert_eq!(vote(body), Reading::NoVote, "{body}");
        }
    }

    #[test]
    fn abstaining() {
        for body in [
            "Abstain",
            "abstain.",
            "I abstain",
            "Abstaining",
            "My vote: abstain",
        ] {
            assert_eq!(vote(body), Reading::Vote(Vote::Abstain), "{body}");
        }
        assert_eq!(vote("Why would anyone abstain?"), Reading::NoVote);
    }

    #[test]
    fn unknown_option() {
        assert_eq!(vote("Option 3"), Reading::UnknownOption(3));
        assert_eq!(vote("Option 1\nOption 7"), Reading::UnknownOption(7));
        // Not said to be an option
        assert_eq!(vote("3"), Reading::NoVote);
    }

    #[test]
    fn votes_that_dont_agree() {
        assert_eq!(vote("Option 1\n\nOption 2"), Reading::Conflicting);
        assert_eq!(vote("Option 1\nAbstain"), Reading::Conflicting);
        assert_eq!(
            vote_of_three("1. Option 2\n1. Option 3"),
            Reading::Conflicting
        );
        assert_eq!(
            vote_of_three("1. Option 2\n2. Option 2"),
            Reading::Conflicting
        );
        assert_eq!(
            vote_of_three("Option 2 > Option 1\nOption 3"),
            Reading::Conflicting
        );
    }

    #[test]
    fn ranking_on_one_line() {
        for body in [
            "Option 2 > Option 1 > Option 3",
            "option 2, option 1, option 3",
            "2 > 1 > 3",
            "2>1>3",
            "2, 1, 3",
            "Option 2 -> Option 1 -> Option 3",
            "Option 2 then option 1 then option 3",
            "My vote: Option 2 > 1 > 3",
        ] {
            assert_eq!(vote_of_three(body), ranked(&[2, 1, 3]), "{body}");
        }
        assert_eq!(vote_of_three("Option 3 > Option 1"), ranked(&[3, 1]));
        assert_eq!(vote_of_three("Option 3 > Option 3"), Reading::NoVote);
        assert_eq!(
            vote_of_three("Option 3 > Option 4"),
            Reading::UnknownOption(4)
        );
    }

    #[test]
    fn ranking_on_numbered_lines() {
        assert_eq!(
            vote_of_three("2. Option 1\n1. Option 2\n3) Option 3"),
            ranked(&[2, 1, 3])
        );
        // Unnumbered lines could be anything
        assert_eq!(vote_of_three("Option 2\nOption 1"), Reading::Conflicting);
    }
}
