//! Reads the votes members wrote under the cut line of their replies

use std::collections::{BTreeMap, HashMap};

use jiff::Timestamp;

use crate::{motion::Motion, roster::Roster};

/// Start of the cut line, eg: `VOTE BELOW THIS LINE`
const CUT_LINE: &str = "vote below";

#[derive(Debug, Clone)]
pub struct Reply {
    pub author: String,
    pub sender: Option<String>,
    pub date: Timestamp,
    pub url: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Vote {
    Ranked(Vec<u32>),
    Abstain,
}

/// A member's vote
#[derive(Debug, Clone, PartialEq)]
pub struct Ballot {
    pub member: String,
    pub vote: Vote,
}

/// A rejected vote
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    pub author: String,
    pub url: String,
    pub reason: Reason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    UnknownSender,
    OutsideWindow,
    Invalid,
}

/// Read a vote from a member's reply
pub fn read(body: &str, motion: &Motion) -> Option<Vote> {
    let line = body
        .lines()
        .skip_while(|line| !line.to_lowercase().contains(CUT_LINE))
        .skip(1)
        .find(|line| {
            !line
                .chars()
                .all(|c| c.is_whitespace() || c == '>' || c == '-')
        })?
        .to_lowercase();

    if line.contains("abstain") {
        return Some(Vote::Abstain);
    }

    // get rid of all whitespaces and any usage of "option"
    // this way "Option 1 > Option 2" also becomes just "1>2"
    let line = line
        .split_whitespace()
        .collect::<String>()
        .replace("option", "");

    let mut options = Vec::new();

    for part in line.split('>') {
        // without this, part.parse() will take something like "+1" as a vote for option 1
        if !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        match part.parse().ok() {
            Some(number) if motion.option(number).is_some() && !options.contains(&number) => {
                options.push(number);
            }
            _ => return None,
        }
    }
    Some(Vote::Ranked(options))
}

/// Each member's last valid vote
pub(crate) fn collect(
    motion: &Motion,
    replies: &[Reply],
    roster: &Roster,
) -> (HashMap<String, Ballot>, Vec<Rejected>) {
    let mut replies: Vec<&Reply> = replies.iter().collect();
    replies.sort_by_key(|reply| reply.date);

    let mut ballots = HashMap::new();
    let mut invalid: BTreeMap<&str, &Reply> = BTreeMap::new();

    let mut rejected = Vec::new();
    let mut reject = |reply: &Reply, reason| {
        rejected.push(Rejected {
            author: reply.author.clone(),
            url: reply.url.clone(),
            reason,
        })
    };

    for reply in replies {
        let member = reply.sender.as_deref().and_then(|s| roster.member_of(s));
        let open = (motion.opens..=motion.closes).contains(&reply.date);
        match (member, read(&reply.body, motion)) {
            (Some(member), Some(vote)) if open => {
                let member = member.to_string();
                ballots.insert(member.clone(), Ballot { member, vote });
            }
            (Some(member), None) if open => {
                invalid.entry(member).or_insert(reply);
            }
            // Anyone can comment. Only a valid vote outside the window is worth reporting
            (Some(_), Some(_)) => reject(reply, Reason::OutsideWindow),
            (None, Some(_)) => reject(reply, Reason::UnknownSender),
            _ => {}
        }
    }

    for (member, reply) in invalid {
        if !ballots.contains_key(member) {
            reject(reply, Reason::Invalid);
        }
    }
    (ballots, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::tests::{MAIL, motion};

    fn quoted(mail: &str) -> String {
        mail.lines().map(|line| format!("> {line}\n")).collect()
    }

    fn vote(text: &str) -> Option<Vote> {
        read(&format!("{}>\n\n{text}\n", quoted(MAIL)), &motion(MAIL))
    }

    fn ranked(options: &[u32]) -> Option<Vote> {
        Some(Vote::Ranked(options.to_vec()))
    }

    fn vote_of_three(text: &str) -> Option<Vote> {
        let three = "Option 1: a\nOption 2: b\nOption 3: c\nVOTE BELOW THIS LINE\n";
        read(&format!("{}\n{text}", quoted(three)), &motion(three))
    }

    #[test]
    fn vote_under_the_quoted_mail() {
        let body = format!(
            "On Saturday, September 12th, 2026 at 04:47, Derek <derek@example.org> wrote:\n\n{}>\n\nOption 1\n\n",
            quoted(MAIL)
        );
        assert_eq!(read(&body, &motion(MAIL)), ranked(&[1]));
    }

    #[test]
    fn vote_under_the_headers_mail_clients_write() {
        let indented: String = MAIL.lines().map(|line| format!(" > {line}\n")).collect();
        for body in [
            format!(
                "On Saturday, September 12th, 2026 at 04:47, Derek <derek@example.org> wrote:\n\n{}\nOption 1\n",
                quoted(MAIL)
            ),
            format!(
                "12 Sept 2026 05:17:44 Derek <derek@example.org>:\n{}\nOption 1\n",
                quoted(MAIL)
            ),
            format!(
                "Sep 12, 2026 1:47:44 AM Derek <derek@example.org>:\n\n{}\nOption 1\n",
                quoted(MAIL)
            ),
            format!(
                "From: Derek <derek@example.org>\nTo: <governance@example.org>\nDate: Sat, 12 Sep 2026 01:47:36 +0200\nSubject: Vote\n\n{indented}\nOption 1\n"
            ),
        ] {
            assert_eq!(read(&body, &motion(MAIL)), ranked(&[1]), "{body}");
        }
    }

    #[test]
    fn mail_quoted_without_a_prefix() {
        let body = format!("From: Derek\nSubject: Vote\n\n{MAIL}\nOption 2\n");
        assert_eq!(read(&body, &motion(MAIL)), ranked(&[2]));
    }

    #[test]
    fn nothing_under_the_cut_line_is_no_vote() {
        let above = format!("Option 2\n\nOn Friday, Derek wrote:\n{}", quoted(MAIL));
        assert_eq!(read(&above, &motion(MAIL)), None);
        assert_eq!(read("Option 2", &motion(MAIL)), None);
    }

    #[test]
    fn only_the_first_line_counts() {
        assert_eq!(vote("Option 1\nOption 2\n-- \nAlice"), ranked(&[1]));
        assert_eq!(vote("Option 1\n\nbecause it scales"), ranked(&[1]));
        assert_eq!(vote("Hi all,\n\nOption 2"), None);
    }

    #[test]
    fn ways_to_write_a_vote() {
        for text in [
            "Option 1",
            "option 1",
            "OPTION 1",
            "Option1",
            "1",
            " 1 ",
            "\u{a0}Option\u{a0}1 ",
        ] {
            assert_eq!(vote(text), ranked(&[1]), "{text}");
        }
    }

    #[test]
    fn anything_else_is_not_a_vote() {
        for text in [
            "",
            "Thanks for organising this!",
            "Option 1.",
            "Option #1",
            "#1",
            "option one",
            "**Option 1**",
            "- Option 1",
            "I vote for Option 1",
            "My vote: Option 1",
            "Option 1: Square-Root Proportional Weighting and Dual-Gate Quorum Model",
            "Option 1 because it scales",
            "Option 1 or Option 2",
            "Option 1/2",
            "Options 1",
            "+1",
            "+2",
            "Option",
            "1 >",
            "1 > > 2",
        ] {
            assert_eq!(vote(text), None, "{text}");
        }
    }

    #[test]
    fn abstaining() {
        for text in [
            "Abstain",
            "ABSTAIN",
            "I abstain",
            "Abstaining.",
            "Why would anyone abstain?",
        ] {
            assert_eq!(vote(text), Some(Vote::Abstain), "{text}");
        }
        assert_eq!(vote("Option 1\nAbstain"), ranked(&[1]));
    }

    #[test]
    fn unknown_option_is_invalid() {
        assert_eq!(vote("Option 3"), None);
        assert_eq!(vote("3"), None);
        assert_eq!(vote("1 > 7"), None);
    }

    #[test]
    fn ranking() {
        for body in [
            "Option 2 > Option 1 > Option 3",
            "Option 2 > 1 > 3",
            "option 2>1>3",
            "2 > 1 > 3",
            "2>1>3",
            " 2 >1>  3 ",
        ] {
            assert_eq!(vote_of_three(body), ranked(&[2, 1, 3]), "{body}");
        }
        assert_eq!(vote_of_three("Option 3 > Option 1"), ranked(&[3, 1]));
        for body in ["3 > 3", "2, 1, 3", "2 -> 1 -> 3", "2 then 1 then 3"] {
            assert_eq!(vote_of_three(body), None, "{body}");
        }
    }
}
