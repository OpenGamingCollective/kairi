//! `/votes`, the count of a vote held on a mailing list: a summary anyone can read at a
//! glance, and a button for who voted what

use std::fmt::Write;

use kameo::error::SendError;
use poise::serenity_prelude as serenity;
use strum::VariantArray;
use voting::{Ballot, Outcome, Reason, Round, Standing, Tally, Vote};

use super::{Data, SerenityContext, SerenityError};
use crate::{
    actors::{Count, CountVotes, FetchList},
    types::MailingListEnum,
};

/// Discord's limits
const TITLE: usize = 256;
const FIELD: usize = 1024;
const FIELDS: usize = 25;
const EMBED: usize = 6000;
const CHOICE: usize = 100;
const CHOICES: usize = 25;

const GREEN: u32 = 0x2e_cc_71;
const ORANGE: u32 = 0xe6_7e_22;
const RED: u32 = 0xe7_4c_3c;

/// Starts the id of the details button, the list and the uid of the thread follow
const DETAILS: &str = "votes";

/// Cells of a result bar
const BAR: usize = 12;

/// A field of an embed: its name, its lines and whether it sits next to the others
type Field = (String, Vec<String>, bool);

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
            let details = serenity::CreateButton::new(details_id(&count))
                .label("Details")
                .style(serenity::ButtonStyle::Secondary);
            let reply = poise::CreateReply::default()
                .embed(summary(&count))
                .components(vec![serenity::CreateActionRow::Buttons(vec![details])]);
            ctx.send(reply).await?;
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

/// Answer the details button, only to who pressed it
///
/// The id of the button says which thread, it still works after a restart
pub async fn on_event(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    data: &Data,
) -> Result<(), SerenityError> {
    let serenity::FullEvent::InteractionCreate { interaction } = event else {
        return Ok(());
    };
    let Some(component) = interaction.as_message_component() else {
        return Ok(());
    };
    let Some((list, uid)) = parse_details_id(&component.data.custom_id) else {
        return Ok(());
    };

    component.defer_ephemeral(&ctx.http).await?;
    let count = CountVotes {
        list,
        thread: uid,
        requested_by: format!("discord /votes details @{}", component.user.name),
    };
    let followup = serenity::CreateInteractionResponseFollowup::new().ephemeral(true);
    let followup = match data.voting.ask(count).await {
        Ok(count) => followup.embed(details(&count)),
        Err(SendError::HandlerError(error)) => {
            followup.content(format!("Can't count the votes: {error}"))
        }
        Err(error) => followup.content(format!("Can't count the votes right now: {error}")),
    };
    component.create_followup(&ctx.http, followup).await?;
    Ok(())
}

fn details_id(count: &Count) -> String {
    format!("{DETAILS}:{}:{}", count.list, count.uid)
}

fn parse_details_id(id: &str) -> Option<(MailingListEnum, String)> {
    let mut parts = id.split(':');
    if parts.next()? != DETAILS {
        return None;
    }
    let list = parts.next()?;
    let list = MailingListEnum::VARIANTS
        .iter()
        .find(|variant| variant.to_string() == list)?;
    Some((*list, parts.next()?.to_string()))
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

fn colour(tally: &Tally) -> u32 {
    match tally.outcome {
        _ if tally.open => ORANGE,
        Outcome::Passed(_) => GREEN,
        Outcome::Tied | Outcome::Void | Outcome::NoVotes => RED,
    }
}

/// The outcome and the score of each option
fn summary(count: &Count) -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title(truncate(&count.name, TITLE))
        .url(&count.url)
        .description(overview(&count.tally))
        .colour(colour(&count.tally))
}

/// Who voted what in each organisation, and what wasn't counted
fn details(count: &Count) -> serenity::CreateEmbed {
    let tally = &count.tally;
    let title = truncate(&count.name, TITLE);
    let footer = format!(
        "{} members, {} of them can vote",
        tally.members, tally.eligible
    );

    // A field too long goes on in the next one, as long as the embed has room
    let size = |text: &str| text.chars().count();
    let mut room = EMBED.saturating_sub(size(&title) + size(&footer));
    let fields: Vec<(String, String, bool)> = fields(tally)
        .into_iter()
        .flat_map(|(name, lines, inline)| {
            let texts = chunks(&lines, FIELD).into_iter().enumerate();
            texts.map(move |(position, text)| match position {
                0 => (name.clone(), text, inline),
                _ => (format!("{name}, continued"), text, inline),
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
        .fields(fields)
        .footer(serenity::CreateEmbedFooter::new(footer))
        .colour(colour(tally))
}

/// Everything the summary says
fn overview(tally: &Tally) -> String {
    let mut text = headline(tally);

    if let Some(round) = tally.rounds.last() {
        let _ = write!(text, "\n```\n{}\n```", bars(round).join("\n"));
        if tally.rounds.len() > 1 {
            let rounds = tally.rounds.len();
            let _ = writeln!(text, "After {rounds} rounds of instant runoff");
        }
        if tally.abstained > 0.0 {
            let abstained = percent(tally.abstained);
            let _ = writeln!(text, "{abstained} of the vote abstained");
        }
    } else {
        text.push('\n');
    }

    let _ = write!(
        text,
        "**{}** ballots, **{}** of **{}** organisations reached quorum",
        tally.ballots.len(),
        tally.counting(),
        tally.standings.len()
    );
    if !tally.rejected.is_empty() {
        let rejected = tally.rejected.len();
        let replies = if rejected == 1 { "reply" } else { "replies" };
        let _ = write!(text, "\n**{rejected}** {replies} not counted");
    }
    text
}

fn headline(tally: &Tally) -> String {
    let outcome = match tally.outcome {
        Outcome::Passed(number) => {
            let title = tally
                .motion
                .option(number)
                .map_or("", |option| option.title.as_str());
            let verb = if tally.open { "is ahead" } else { "passed" };
            format!("**Option {number} {verb}**\n{title}")
        }
        Outcome::Tied if tally.open => "**Tied** so far".to_string(),
        Outcome::Tied => "**Tied**, the vote has to be held again".to_string(),
        Outcome::Void if tally.open => "**No quorum** yet".to_string(),
        Outcome::Void => "**Void**, not enough organisations reached quorum".to_string(),
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

/// A bar for each option of a round, aligned for a code block
fn bars(round: &Round) -> Vec<String> {
    let total = round.total();
    round
        .scores
        .iter()
        .map(|(number, score)| {
            let share = if total > 0.0 { score / total } else { 0.0 };
            let filled = ((share * BAR as f64).round() as usize).min(BAR);
            let bar = "█".repeat(filled) + &"░".repeat(BAR - filled);
            format!("Option {number}  {bar}  {:>6}", percent(share))
        })
        .collect()
}

/// The fields of the details
fn fields(tally: &Tally) -> Vec<Field> {
    let mut fields: Vec<Field> = Vec::new();

    if tally.rounds.len() > 1 {
        fields.push(("Runoff".to_string(), rounds(tally), false));
    }

    // One card per organisation
    fields.extend(tally.standings.iter().map(|standing| card(tally, standing)));

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
        fields.push(("Notes".to_string(), notes, false));
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
        fields.push((name, rejected.collect(), false));
    }
    fields
}

/// An organisation: its turnout, its share of the vote and what its members voted
fn card(tally: &Tally, standing: &Standing) -> Field {
    let mark = if standing.counts { "✅" } else { "❌" };
    let turnout = format!("{}/{} voted", standing.voted, standing.roster);
    let mut lines = vec![if standing.counts {
        format!("{turnout}, {} of the vote", percent(standing.weight))
    } else if standing.voted > 0 {
        format!("{turnout}, under quorum")
    } else {
        turnout
    }];

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
        let label = match vote {
            Vote::Abstain => "Abstained".to_string(),
            Vote::Ranked(options) => {
                let options: Vec<String> = options.iter().map(u32::to_string).collect();
                format!("Option {}", options.join(" > "))
            }
        };
        let members: Vec<&str> = cast
            .iter()
            .filter(|ballot| ballot.vote == *vote)
            .map(|ballot| ballot.member.as_str())
            .collect();
        lines.push(format!("**{label}**: {}", members.join(", ")));
    }
    (format!("{mark} {}", standing.name), lines, true)
}

/// Each round of an instant runoff
fn rounds(tally: &Tally) -> Vec<String> {
    tally
        .rounds
        .iter()
        .enumerate()
        .map(|(position, round)| {
            let total = round.total();
            let scores: Vec<String> = round
                .scores
                .iter()
                .map(|(number, score)| {
                    let share = if total > 0.0 { score / total } else { 0.0 };
                    format!("Option {number} **{}**", percent(share))
                })
                .collect();
            let mut line = format!("Round {}: {}", position + 1, scores.join(", "));
            if !round.eliminated.is_empty() {
                let out: Vec<String> = round.eliminated.iter().map(u32::to_string).collect();
                let _ = write!(line, ", option {} out", out.join(" and "));
            }
            line
        })
        .collect()
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
    const THREE: &str = "Option 1: a\nOption 2: b\nOption 3: c\n";

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
                    "erin": { "email": "" },
                    "frank": { "email": "frank@example.org" },
                    "grace": { "email": "grace@example.org" },
                    "heidi": { "email": "heidi@example.org" }
                },
                "groups": {
                    "fyra": { "description": "Fyra Labs", "org": true, "members": ["alice", "bob"] },
                    "nobara": { "description": "Nobara", "org": true, "members": ["frank", "grace", "heidi"] },
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

    fn count_of(mail: &str, replies: &[Reply], hours: i64) -> Tally {
        let now = sent() + TimeDelta::hours(hours);
        voting::tally(mail, sent(), replies, &roster(), &Policy::default(), now).unwrap()
    }

    fn count(replies: &[Reply], hours: i64) -> Tally {
        count_of(MAIL, replies, hours)
    }

    fn field<'a>(fields: &'a [Field], name: &str) -> &'a [String] {
        let (_, lines, _) = fields.iter().find(|field| field.0 == name).unwrap();
        lines
    }

    /// fyra votes together, nobara is under its quorum, a stranger votes too
    fn closed() -> Tally {
        count(
            &[
                reply("alice", "Option 1"),
                reply("bob", "Option 1"),
                reply("dave", "Option 2"),
                reply("frank", "Option 2"),
                reply("mallory", "Option 2"),
            ],
            48,
        )
    }

    #[test]
    fn the_summary_is_the_outcome_and_the_scores() {
        // √2 for fyra, 1 for dave alone in the unaffiliated block
        assert_eq!(
            overview(&closed()),
            "**Option 1 passed**\n\
             Square roots\n\
             Closed <t:1789256856:R>\n\
             ```\n\
             Option 1  ███████░░░░░   58.6%\n\
             Option 2  █████░░░░░░░   41.4%\n\
             ```\
             **4** ballots, **2** of **4** organisations reached quorum\n\
             **1** reply not counted"
        );
    }

    #[test]
    fn an_open_vote_is_provisional() {
        let tally = count(&[reply("alice", "Option 2"), reply("dave", "abstain")], 2);
        let overview = overview(&tally);
        assert!(overview.starts_with("**Option 2 is ahead**\nEqual shares\nOpen until <t:"));
        assert!(overview.contains("Option 2  ████████████  100.0%\n"));
        assert!(overview.contains("```41.4% of the vote abstained\n**2** ballots"));
    }

    #[test]
    fn a_void_vote_has_no_scores() {
        let tally = count(&[reply("carol", "Option 2")], 48);
        assert_eq!(
            overview(&tally),
            "**Void**, not enough organisations reached quorum\n\
             Closed <t:1789256856:R>\n\
             **1** ballots, **1** of **4** organisations reached quorum"
        );
    }

    #[test]
    fn the_details_have_a_card_per_organisation() {
        let fields = fields(&closed());
        let names: Vec<&str> = fields.iter().map(|field| &*field.0).collect();
        assert_eq!(
            names,
            [
                "✅ Fyra Labs",
                "❌ Nobara",
                "❌ Universal Blue",
                "✅ Unaffiliated",
                "Notes",
                "Not counted (1)",
            ]
        );
        // Cards sit side by side, the rest takes the whole width
        let inline: Vec<bool> = fields.iter().map(|field| field.2).collect();
        assert_eq!(inline, [true, true, true, true, false, false]);

        assert_eq!(
            field(&fields, "✅ Fyra Labs"),
            ["2/2 voted, 58.6% of the vote", "**Option 1**: alice, bob"]
        );
        assert_eq!(
            field(&fields, "❌ Nobara"),
            ["1/3 voted, under quorum", "**Option 2**: frank"]
        );
        assert_eq!(field(&fields, "❌ Universal Blue"), ["0/2 voted"]);
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
    fn a_runoff_shows_its_rounds() {
        let tally = count_of(
            THREE,
            &[
                reply("alice", "Option 1 > Option 2"),
                reply("bob", "Option 3 > Option 2"),
                reply("dave", "Option 2"),
                reply("carol", "Abstain"),
            ],
            48,
        );
        assert!(overview(&tally).contains("```After 2 rounds of instant runoff\n"));

        let fields = fields(&tally);
        let rounds = field(&fields, "Runoff");
        assert_eq!(rounds.len(), 2);
        assert!(rounds[0].starts_with("Round 1: Option 1 **"));
        assert!(rounds[0].ends_with(", option 1 and 3 out"));
        assert_eq!(
            field(&fields, "✅ Fyra Labs")[1..],
            ["**Option 1 > 2**: alice", "**Option 3 > 2**: bob"]
        );
    }

    #[test]
    fn the_details_button_remembers_its_thread() {
        assert_eq!(
            parse_details_id("votes:governance:0a1b2c3d4e5f"),
            Some((MailingListEnum::Governance, "0a1b2c3d4e5f".to_string()))
        );
        assert_eq!(parse_details_id("votes:nope:0a1b2c3d4e5f"), None);
        assert_eq!(parse_details_id("other:governance:0a1b"), None);
        assert_eq!(parse_details_id("votes:governance"), None);
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
