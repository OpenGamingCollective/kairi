//! `/votes`, the count of a vote held on a mailing list

use std::fmt::Write;

use kameo::error::SendError;
use poise::serenity_prelude as serenity;
use voting::{Ballot, Outcome, Reason, Round, Tally, Vote};

use super::{SerenityContext, SerenityError};
use crate::{
    actors::{Count, CountVotes, FetchList},
    types::MailingListEnum,
};

/// Discord's limits
const TITLE: usize = 256;
const FIELD: usize = 1024;
const FIELDS: usize = 25;
const EMBED: usize = 6000;

/// Discord trims the spaces a line starts with, not this one
const INDENT: &str = "\u{2003}";
const CHOICE: usize = 100;
const CHOICES: usize = 25;

const GREEN: u32 = 0x2e_cc_71;
const ORANGE: u32 = 0xe6_7e_22;
const RED: u32 = 0xe7_4c_3c;

/// Count the votes of a mailing list thread
#[poise::command(slash_command)]
pub async fn votes(
    ctx: SerenityContext<'_>,
    #[description = "Thread of the vote, or the link of any of its mails"]
    #[autocomplete = "threads"]
    thread: String,
    #[description = "List the thread is on, governance by default"] list: Option<MailingListEnum>,
) -> Result<(), SerenityError> {
    // Reading the mails can take longer than Discord waits
    ctx.defer().await?;

    let count = CountVotes {
        list: list.unwrap_or(MailingListEnum::Governance),
        thread,
        requested_by: format!("discord /votes @{}", ctx.author().name),
    };
    match ctx.data().voting.ask(count).await {
        Ok(count) => {
            ctx.send(poise::CreateReply::default().embed(embed(&count)))
                .await?;
        }
        Err(SendError::HandlerError(error)) => {
            ctx.say(format!("Can't count the votes: {error}")).await?;
        }
        Err(error) => {
            ctx.say(format!("Can't count the votes right now: {error}"))
                .await?;
        }
    }
    Ok(())
}

/// The governance threads, the newest first
async fn threads(ctx: SerenityContext<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let fetch = FetchList {
        list: MailingListEnum::Governance,
        requested_by: format!("discord /votes autocomplete @{}", ctx.author().name),
    };
    let list = ctx.data().reader.ask(fetch).await.unwrap_or_default();

    let partial = partial.to_lowercase();
    let choices = list
        .threads
        .iter()
        .rev()
        .filter(|thread| thread.name.to_lowercase().contains(&partial))
        .take(CHOICES)
        .map(|thread| {
            let name = format!("{} ({})", thread.name, thread.date.format("%Y-%m-%d"));
            serenity::AutocompleteChoice::new(truncate(&name, CHOICE), thread.uid.clone())
        })
        .collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

fn embed(count: &Count) -> serenity::CreateEmbed {
    let tally = &count.tally;
    let colour = match tally.outcome {
        _ if tally.open => ORANGE,
        Outcome::Passed(_) => GREEN,
        Outcome::Tied | Outcome::Void | Outcome::NoVotes => RED,
    };

    let title = truncate(&count.name, TITLE);
    let description = headline(tally);
    let footer = format!(
        "{} members, {} of them can vote",
        tally.members, tally.eligible
    );

    // A field too long goes on in the next one, as long as the embed has room
    let size = |text: &str| text.chars().count();
    let mut room = EMBED.saturating_sub(size(&title) + size(&description) + size(&footer));
    let fields: Vec<(String, String, bool)> = fields(tally)
        .into_iter()
        .flat_map(|(name, lines)| {
            let texts = chunks(&lines, FIELD).into_iter().enumerate();
            texts.map(move |(position, text)| match position {
                0 => (name.clone(), text, false),
                _ => (format!("{name}, continued"), text, false),
            })
        })
        .take(FIELDS)
        .take_while(|(name, text, _)| {
            let fits = size(name) + size(text) <= room;
            if fits {
                room -= size(name) + size(text);
            }
            fits
        })
        .collect();

    serenity::CreateEmbed::new()
        .title(title)
        .url(&count.url)
        .description(description)
        .fields(fields)
        .footer(serenity::CreateEmbedFooter::new(footer))
        .colour(colour)
}

fn headline(tally: &Tally) -> String {
    let outcome = match tally.outcome {
        Outcome::Passed(number) => {
            let title = tally
                .motion
                .option(number)
                .map_or("", |option| option.title.as_str());
            let verb = if tally.open { "is ahead" } else { "passed" };
            format!("**Option {number} {verb}**: {title}")
        }
        Outcome::Tied if tally.open => "**Tied** so far".to_string(),
        Outcome::Tied => "**Tied**, the vote has to be held again".to_string(),
        Outcome::Void if tally.open => "**No quorum** yet".to_string(),
        Outcome::Void => "**Void**, the coalition quorum was not met".to_string(),
        Outcome::NoVotes => "**No votes** for any option".to_string(),
    };

    let closes = tally.motion.closes.timestamp();
    let window = if tally.open {
        format!("Open until <t:{closes}:f>, <t:{closes}:R>, the count can still change")
    } else {
        format!("Closed <t:{closes}:R>")
    };
    format!("{outcome}\n{window}")
}

/// Name and lines of each field
fn fields(tally: &Tally) -> Vec<(String, Vec<String>)> {
    let mut fields = Vec::new();

    if !tally.rounds.is_empty() {
        fields.push(("Result".to_string(), result(tally)));
    }

    let name = format!("Ballots ({})", tally.ballots.len());
    fields.push((name, ballots(tally)));

    let notes: Vec<String> = tally
        .ballots
        .iter()
        .filter(|ballot| !ballot.to_first_mail)
        .map(|ballot| {
            let member = &ballot.member;
            format!("{member} replied to another reply, not to the first mail")
        })
        .collect();
    if !notes.is_empty() {
        fields.push(("Notes".to_string(), notes));
    }

    if !tally.rejected.is_empty() {
        let name = format!("Not counted ({})", tally.rejected.len());
        let rejected = tally.rejected.iter().map(|rejected| {
            let reason = match rejected.reason {
                Reason::UnknownSender => "voted from an address that isn't on the roster".into(),
                Reason::OutsideWindow => "voted while the vote wasn't open".into(),
                Reason::NoVote => "replied without a vote that could be read".into(),
                Reason::Conflicting => "wrote several votes that don't agree".into(),
                Reason::UnknownOption(number) => {
                    format!("voted for option {number}, there is none")
                }
            };
            format!("[{}]({}) {reason}", rejected.author, rejected.url)
        });
        fields.push((name, rejected.collect()));
    }
    fields
}

fn result(tally: &Tally) -> Vec<String> {
    let scores = |round: &Round| -> Vec<String> {
        let total = round.total();
        round
            .scores
            .iter()
            .map(|(number, score)| {
                let share = if total > 0.0 { score / total } else { 0.0 };
                format!("Option {number}: **{}**", percent(share))
            })
            .collect()
    };

    let mut lines = match tally.rounds.as_slice() {
        [round] => scores(round),
        rounds => rounds
            .iter()
            .enumerate()
            .map(|(position, round)| {
                let mut line = format!("Round {}: {}", position + 1, scores(round).join(", "));
                if !round.eliminated.is_empty() {
                    let out: Vec<String> = round.eliminated.iter().map(u32::to_string).collect();
                    let _ = write!(line, ", option {} out", out.join(" and "));
                }
                line
            })
            .collect(),
    };
    if tally.abstained > 0.0 {
        lines.push(format!(
            "{} of the vote abstained",
            percent(tally.abstained)
        ));
    }
    lines
}

/// Each organisation with its turnout and its share of the vote, then what its members voted
fn ballots(tally: &Tally) -> Vec<String> {
    let label = |vote: &Vote| match vote {
        Vote::Abstain => "Abstained".to_string(),
        Vote::Ranked(options) => {
            let options: Vec<String> = options.iter().map(u32::to_string).collect();
            format!("Option {}", options.join(" > "))
        }
    };

    let mut lines = vec![format!(
        "{} of {} count, {} needed",
        tally.counting(),
        tally.standings.len(),
        percent(tally.coalition_quorum)
    )];
    for standing in &tally.standings {
        let turnout = format!("{}/{}", standing.voted, standing.roster);
        lines.push(if standing.counts {
            let weight = percent(standing.weight);
            format!("✅ {}, {turnout}, {weight}", standing.name)
        } else {
            format!("❌ {}, {turnout}", standing.name)
        });

        let cast: Vec<&Ballot> = tally
            .ballots
            .iter()
            .filter(|ballot| standing.voters.contains(&ballot.member))
            .collect();
        // In the order of the motion, the abstentions last
        let mut votes: Vec<&Vote> = Vec::new();
        for ballot in &cast {
            if !votes.contains(&&ballot.vote) {
                votes.push(&ballot.vote);
            }
        }
        votes.sort_by_key(|vote| match vote {
            Vote::Ranked(options) => (0, options.clone()),
            Vote::Abstain => (1, Vec::new()),
        });

        for vote in votes {
            let members: Vec<&str> = cast
                .iter()
                .filter(|ballot| ballot.vote == *vote)
                .map(|ballot| ballot.member.as_str())
                .collect();
            lines.push(format!("{INDENT}{}: {}", label(vote), members.join(", ")));
        }
    }
    lines
}

fn percent(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

/// Whole lines gathered in texts of at most `limit` characters each
fn chunks(lines: &[String], limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut text = String::new();
    for line in lines {
        // A line longer than a field, better cut than left out
        let line = truncate(line, limit);
        if !text.is_empty() && text.chars().count() + 1 + line.chars().count() > limit {
            chunks.push(std::mem::take(&mut text));
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
    }
    if !text.is_empty() {
        chunks.push(text);
    }
    chunks
}

/// At most `limit` characters
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(limit.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta, Utc};
    use voting::{Policy, Reply, Roster};

    use super::*;

    const MAIL: &str = "Option 1: Square roots\nOption 2: Equal shares\nVOTE BELOW THIS LINE\n";

    fn sent() -> DateTime<Utc> {
        "2026-09-11T23:47:36Z".parse().unwrap()
    }

    fn roster() -> Roster {
        Roster::parse(
            r#"{
                "members": {
                    "alice": { "email": "alice@example.org" },
                    "bob": { "email": "bob@example.org" },
                    "carol": { "email": "carol@example.org" },
                    "dave": { "email": "dave@example.org" },
                    "erin": { "email": "" }
                },
                "groups": {
                    "fyra": { "description": "Fyra Labs", "org": true, "members": ["alice", "bob"] },
                    "ublue": { "description": "Universal Blue", "org": true, "members": ["carol", "erin"] }
                }
            }"#,
        )
        .unwrap()
    }

    fn reply(author: &str, body: &str) -> Reply {
        Reply {
            author: author.to_string(),
            sender: Some(format!("{author}@example.org")),
            date: sent() + TimeDelta::hours(1),
            url: format!("https://l/{author}/"),
            to_first_mail: author != "bob",
            body: body.to_string(),
        }
    }

    fn count(replies: &[Reply], hours: i64) -> Tally {
        let now = sent() + TimeDelta::hours(hours);
        voting::tally(MAIL, sent(), replies, &roster(), &Policy::default(), now).unwrap()
    }

    fn field<'a>(fields: &'a [(String, Vec<String>)], name: &str) -> &'a [String] {
        let (_, lines) = fields.iter().find(|field| field.0 == name).unwrap();
        lines
    }

    #[test]
    fn a_closed_vote_shows_who_voted_what() {
        let tally = count(
            &[
                reply("alice", "Option 1"),
                reply("bob", "Option 1"),
                reply("dave", "Option 2"),
                reply("mallory", "Option 2"),
            ],
            48,
        );

        assert_eq!(
            headline(&tally),
            "**Option 1 passed**: Square roots\nClosed <t:1789256856:R>"
        );

        let fields = fields(&tally);
        // √2 for fyra, 1 for dave alone in the unaffiliated block
        assert_eq!(
            field(&fields, "Result"),
            ["Option 1: **58.6%**", "Option 2: **41.4%**"]
        );
        assert_eq!(
            field(&fields, "Ballots (3)"),
            [
                "2 of 3 count, 50.0% needed",
                "✅ Fyra Labs, 2/2, 58.6%",
                "\u{2003}Option 1: alice, bob",
                "❌ Universal Blue, 0/2",
                "✅ Unaffiliated, 1/1, 41.4%",
                "\u{2003}Option 2: dave",
            ]
        );
        assert_eq!(
            field(&fields, "Notes"),
            ["bob replied to another reply, not to the first mail"]
        );
        assert_eq!(
            field(&fields, "Not counted (1)"),
            ["[mallory](https://l/mallory/) voted from an address that isn't on the roster"]
        );
    }

    #[test]
    fn an_open_vote_is_provisional() {
        let tally = count(&[reply("alice", "Option 2"), reply("dave", "abstain")], 2);
        let headline = headline(&tally);
        assert!(headline.starts_with("**Option 2 is ahead**: Equal shares\nOpen until <t:"));

        let fields = fields(&tally);
        assert_eq!(
            field(&fields, "Result"),
            [
                "Option 1: **0.0%**",
                "Option 2: **100.0%**",
                "41.4% of the vote abstained",
            ]
        );
    }

    #[test]
    fn a_void_vote_has_no_result() {
        let tally = count(&[reply("carol", "Option 2")], 48);
        assert!(headline(&tally).starts_with("**Void**"));

        let fields = fields(&tally);
        assert!(fields.iter().all(|(name, _)| name != "Result"));
        assert!(fields.iter().all(|(name, _)| name != "Notes"));
        assert_eq!(
            field(&fields, "Ballots (1)"),
            [
                "1 of 3 count, 50.0% needed",
                "❌ Fyra Labs, 0/2",
                "✅ Universal Blue, 1/2, 100.0%",
                "\u{2003}Option 2: carol",
                "❌ Unaffiliated, 0/1",
            ]
        );
    }

    #[test]
    fn long_fields_go_on_in_the_next_one() {
        let lines: Vec<String> = (0..5).map(|line| format!("line {line}")).collect();
        assert_eq!(chunks(&lines[..2], 100), ["line 0\nline 1"]);
        assert_eq!(
            chunks(&lines, 13),
            ["line 0\nline 1", "line 2\nline 3", "line 4"]
        );
        assert_eq!(chunks(&lines[..1], 5), ["line…"]);
        assert!(chunks(&[], 5).is_empty());

        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello!", 5), "hell…");
    }
}
