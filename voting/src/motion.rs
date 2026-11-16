//! What's being voted on, read from the mail that opens the thread

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;

/// Start of the cut line, as in `VOTE BELOW THIS LINE`
const CUT_LINE: &str = "vote below";

/// Lines of the first mail shorter than this could be votes, so they don't mark a quote
const QUOTE_LENGTH: usize = 25;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MotionError {
    #[error("the first mail of the thread doesn't list at least two `Option <n>: <title>`")]
    NoOptions,
}

/// An `Option <number>: <title>` line of the first mail
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub number: u32,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Motion {
    pub options: Vec<Choice>,
    pub opens: DateTime<Utc>,
    pub closes: DateTime<Utc>,
    /// The cut line, normalised
    pub(crate) cut_line: Option<String>,
    /// The body, normalised, to spot it when it's quoted without `>`
    text: String,
}

impl Motion {
    /// Read the first mail of a thread. The vote stays open as long as the mail says (like
    /// `open for 24 hours`), or for `window` if it doesn't say
    pub fn read(body: &str, sent: DateTime<Utc>, window: TimeDelta) -> Result<Self, MotionError> {
        let mut options: Vec<Choice> = Vec::new();
        for line in body.lines() {
            if let Some(option) = option_of(line)
                && !options.iter().any(|known| known.number == option.number)
            {
                options.push(option);
            }
        }
        if options.len() < 2 {
            return Err(MotionError::NoOptions);
        }

        let text = fold(body);
        let cut_line = body.lines().map(fold).find(|line| line.contains(CUT_LINE));

        Ok(Self {
            options,
            opens: sent,
            closes: sent + window_of(&text).unwrap_or(window),
            cut_line,
            text,
        })
    }

    pub fn option(&self, number: u32) -> Option<&Choice> {
        self.options.iter().find(|option| option.number == number)
    }

    /// Whether a line in a reply is one of the first mail's long lines
    pub(crate) fn quotes(&self, line: &str) -> bool {
        let line = fold(line);
        line.len() >= QUOTE_LENGTH && self.text.contains(&line)
    }
}

/// Lowercase, with each run of whitespace collapsed to one space
pub(crate) fn fold(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `Option 1: Title`, where the separator can also be `-`, `.` or `)`
fn option_of(line: &str) -> Option<Choice> {
    let line = line.trim();
    let rest = line
        .get(..6)
        .filter(|word| word.eq_ignore_ascii_case("option"))
        .map(|_| line[6..].trim_start())?;

    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let number = rest[..digits].parse().ok()?;
    let title = rest[digits..]
        .trim_start()
        .strip_prefix([':', '-', '.', ')'])?
        .trim();

    (!title.is_empty()).then(|| Choice {
        number,
        title: title.to_string(),
    })
}

/// `open for <n> hours`, or days, or weeks
fn window_of(text: &str) -> Option<TimeDelta> {
    let mut words = text.split("open for ").nth(1)?.split(' ');
    let count: i64 = words.next()?.parse().ok()?;
    let unit = words.next()?;

    let hours = if unit.starts_with("hour") {
        1
    } else if unit.starts_with("day") {
        24
    } else if unit.starts_with("week") {
        24 * 7
    } else {
        return None;
    };
    TimeDelta::try_hours(count.checked_mul(hours)?).filter(|window| *window > TimeDelta::zero())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const MAIL: &str = "This email initiates voting on what voting system we will adopt as an \n\
        organization going forward. Two proposals have been put forward.\n\
        \n\
        Voting Rules:\n\
        - Voting will be open for 24 hours starting at the send time of this email.\n\
        \n\
        Voting Action:\n\
        - Place your choice, either Option 1 or Option 2, BELOW the cut line at \n\
        the BOTTOM of this email.\n\
        \n\
        Option 1: Square-Root Proportional Weighting and Dual-Gate Quorum Model\n\
        \n\
        This proposal establishes a voting system that balances organizational \n\
        representation while protecting smaller member groups.\n\
        \n\
        Option 2: Size Scaled and Equal Distribution Model\n\
        \n\
        The voting power each organization corresponds to the highest number of \n\
        individuals any one organization has.\n\
        \n\
        VOTE BELOW THIS LINE\n\
        ---\n";

    pub fn sent() -> DateTime<Utc> {
        "2026-09-11T23:47:36Z".parse().unwrap()
    }

    pub fn motion(body: &str) -> Motion {
        Motion::read(body, sent(), TimeDelta::hours(24)).unwrap()
    }

    #[test]
    fn options_are_the_titled_lines() {
        let motion = motion(MAIL);
        assert_eq!(
            motion.options,
            [
                Choice {
                    number: 1,
                    title: "Square-Root Proportional Weighting and Dual-Gate Quorum Model"
                        .to_string()
                },
                Choice {
                    number: 2,
                    title: "Size Scaled and Equal Distribution Model".to_string()
                },
            ]
        );
        assert_eq!(motion.option(2).unwrap().number, 2);
        assert!(motion.option(3).is_none());
    }

    #[test]
    fn an_option_is_only_listed_once() {
        let motion = motion("Option 1: a\nOPTION 2 - b\noption 1: again\nOption 3) c\n");
        let numbers: Vec<u32> = motion.options.iter().map(|option| option.number).collect();
        assert_eq!(numbers, [1, 2, 3]);
        assert_eq!(motion.options[0].title, "a");
    }

    #[test]
    fn a_mail_without_two_options_is_not_a_vote() {
        let read = |body| Motion::read(body, sent(), TimeDelta::hours(24)).map(|_| ());
        assert_eq!(read("hello"), Err(MotionError::NoOptions));
        assert_eq!(read("Option 1: alone"), Err(MotionError::NoOptions));
        assert_eq!(
            read("Option 1\nOption 2\nOptional: x"),
            Err(MotionError::NoOptions)
        );
    }

    #[test]
    fn the_vote_lasts_what_the_mail_says() {
        assert_eq!(motion(MAIL).closes - sent(), TimeDelta::hours(24));

        let days = motion("Voting is OPEN  for 3\ndays.\nOption 1: a\nOption 2: b");
        assert_eq!(days.closes - sent(), TimeDelta::days(3));
    }

    #[test]
    fn the_vote_lasts_the_default_window_otherwise() {
        let read = |body| Motion::read(body, sent(), TimeDelta::hours(48)).unwrap();
        let unsaid = read("Option 1: a\nOption 2: b");
        assert_eq!(unsaid.closes - sent(), TimeDelta::hours(48));

        let unreadable = read("open for a while, open for 0 hours\nOption 1: a\nOption 2: b");
        assert_eq!(unreadable.closes - sent(), TimeDelta::hours(48));
    }

    #[test]
    fn the_cut_line_is_found() {
        assert_eq!(
            motion(MAIL).cut_line.as_deref(),
            Some("vote below this line")
        );
        assert!(motion("Option 1: a\nOption 2: b").cut_line.is_none());
    }

    #[test]
    fn long_lines_of_the_mail_are_recognised() {
        let motion = motion(MAIL);
        assert!(
            motion.quotes("  organization going forward.   Two proposals have been put forward.")
        );
        assert!(motion.quotes("OPTION 2: Size Scaled and Equal Distribution Model"));
        // Could be a vote
        assert!(!motion.quotes("Option 1"));
        assert!(!motion.quotes("I'd rather have something else entirely"));
    }
}
