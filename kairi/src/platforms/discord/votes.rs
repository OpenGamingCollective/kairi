//! `/votes` shows a summary of a mailing list vote, with a button to see who voted what

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

/// Prefix of the details button's id, which looks like `votes:<list>:<uid>`
const DETAILS: &str = "votes";

/// Width of a result bar, in cells
const BAR: usize = 12;

/// An embed field as (name, lines, inline)
type Field = (String, Vec<String>, bool);

/// Count the votes on a mailing list thread
#[poise::command(slash_command)]
pub async fn votes(
    ctx: SerenityContext<'_>,
    #[description = "Thread of the vote, or the link of any of its mails"]
    #[autocomplete = "threads"]
    thread: String,
    #[description = "List the thread is on, governance by default"] list: Option<MailingListEnum>,
) -> Result<(), SerenityError> {
    // Reading the mails can take longer than Discord is willing to wait
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

/// Reply to the details button, visible only to whoever pressed it
///
/// The button id carries the thread, so the button still works after a restart
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

/// Governance threads, newest first
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
        Outcome::NoMajority | Outcome::Tied | Outcome::Void | Outcome::NoVotes => RED,
    }
}

/// The outcome and each option's score
fn summary(count: &Count) -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title(truncate(&count.name, TITLE))
        .url(&count.url)
        .description(overview(&count.tally))
        .colour(colour(&count.tally))
}

/// Who voted for what in each organisation, plus anything that wasn't counted
fn details(count: &Count) -> serenity::CreateEmbed {
    let tally = &count.tally;
    let title = truncate(&count.name, TITLE);
    let footer = format!(
        "{} members, {} of them can vote",
        tally.members, tally.eligible
    );

    // Long fields spill over into the next one as long as the embed has room
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

/// Everything that goes into the summary
fn overview(tally: &Tally) -> String {
    let mut text = headline(tally);

    if let Some(round) = tally.rounds.last() {
        let _ = write!(text, "\n```\n{}\n```", bars(&rows(tally, round)).join("\n"));
        let _ = writeln!(
            text,
            "Share of the weight of the organisations that reached quorum"
        );
        if tally.rounds.len() > 1 {
            let rounds = tally.rounds.len();
            let _ = writeln!(text, "After {rounds} rounds of instant runoff");
        }
        text.push('\n');

        let leader = match tally.outcome {
            Outcome::Passed(number) => round.scores.iter().find(|(option, _)| *option == number),
            Outcome::NoMajority => round.scores.iter().max_by(|a, b| a.1.total_cmp(&b.1)),
            _ => None,
        };
        if let Some((number, score)) = leader
            && round.total > 0.0
        {
            let share = percent(score / round.total);
            let _ = writeln!(
                text,
                "Option {number} has **{share}**, more than half is needed"
            );
        }
    } else {
        text.push('\n');
    }

    let entities = tally.standings.len();
    // Smallest number of entities that meets the coalition quorum
    let needed = (tally.coalition_quorum * entities as f64 - 1e-9)
        .ceil()
        .max(0.0) as usize;
    // For auditing: counted ballots, abstentions, then the rest (under quorum or rejected)
    let counted = |abstains: bool| {
        let is = |ballot: &&Ballot| ballot.counted && (ballot.vote == Vote::Abstain) == abstains;
        tally.ballots.iter().filter(is).count()
    };
    let (valid, abstained) = (counted(false), counted(true));
    let left_out = tally.ballots.len() - valid - abstained + tally.rejected.len();
    let _ = write!(
        text,
        "Quorum: **{}** of **{entities}** organisations, **{needed}** needed\n\
         **{valid}** valid {}, **{abstained}** abstained, **{left_out}** not counted",
        tally.counting(),
        plural(valid, "vote", "votes"),
    );
    text
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
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
        Outcome::NoMajority if tally.open => "**No majority** so far".to_string(),
        Outcome::NoMajority => {
            "**No majority**, no option has more than half of the vote".to_string()
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

/// Where the counted weight went: options still in the race, abstentions or exhausted ballots
///
/// `round` is the final round. Options eliminated earlier show the round they went out in
fn rows(tally: &Tally, round: &Round) -> Vec<(String, Row)> {
    let abstained = abstained_in_majority(tally);
    let share = |weight: f64| {
        if round.total > 0.0 {
            weight / round.total
        } else {
            0.0
        }
    };
    let mut rows: Vec<(String, Row)> = tally
        .motion
        .options
        .iter()
        .filter_map(|option| {
            let score = round
                .scores
                .iter()
                .find(|(number, _)| *number == option.number);
            let row = match score {
                Some((_, score)) => Row::Share(share(*score)),
                None => {
                    let out = |round: &Round| round.eliminated.contains(&option.number);
                    Row::Out(tally.rounds.iter().position(out)? + 1)
                }
            };
            Some((format!("Option {}", option.number), row))
        })
        .collect();
    // Skip the abstentions row when the policy leaves them out of the majority
    let left_out_by_policy = tally.abstained - abstained >= 0.0005;
    if !left_out_by_policy {
        rows.push(("Abstained".to_string(), Row::Share(share(abstained))));
    }
    let exhausted = share(round.total - abstained - round.voted());
    if exhausted >= 0.0005 {
        rows.push(("No option left".to_string(), Row::Share(exhausted)));
    }
    rows
}

/// One line of the summary
enum Row {
    /// Its share of the weight that counts
    Share(f64),
    /// The runoff round the option was eliminated in
    Out(usize),
}

/// One bar per row, aligned to fit in a code block
fn bars(rows: &[(String, Row)]) -> Vec<String> {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(label, row)| match row {
            Row::Share(share) => {
                let filled = ((share * BAR as f64).round() as usize).min(BAR);
                let bar = "█".repeat(filled) + &"░".repeat(BAR - filled);
                format!("{label:<width$}  {bar}  {:>6}", percent(*share))
            }
            Row::Out(round) => format!("{label:<width$}  eliminated in round {round}"),
        })
        .collect()
}

/// Fields of the details view
fn fields(tally: &Tally) -> Vec<Field> {
    let mut fields: Vec<Field> = Vec::new();

    if tally.rounds.len() > 1 {
        fields.push(("Runoff".to_string(), rounds(tally), false));
    }

    // One card per organisation
    fields.extend(tally.standings.iter().map(|standing| card(tally, standing)));

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

/// An organisation's turnout, weight and votes
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
    // Same order as the motion, with abstentions last
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

/// Every runoff round: each option's score and ballots (`+` marks a ballot that just moved
/// there), followed by the abstentions counted in the majority
fn rounds(tally: &Tally) -> Vec<String> {
    let ballots: Vec<(&str, &[u32])> = tally
        .ballots
        .iter()
        .filter(|ballot| ballot.counted)
        .filter_map(|ballot| match &ballot.vote {
            Vote::Ranked(options) => Some((ballot.member.as_str(), options.as_slice())),
            Vote::Abstain => None,
        })
        .collect();

    let abstained = abstained_in_majority(tally);
    let abstaining: Vec<&str> = tally
        .ballots
        .iter()
        .filter(|ballot| ballot.counted && ballot.vote == Vote::Abstain)
        .map(|ballot| ballot.member.as_str())
        .collect();

    let mut lines = Vec::new();
    // Which option each ballot backed in the previous round
    let mut before: Vec<Option<u32>> = Vec::new();
    for (position, round) in tally.rounds.iter().enumerate() {
        let running = |option: &u32| round.scores.iter().any(|(number, _)| number == option);
        let choices: Vec<Option<u32>> = ballots
            .iter()
            .map(|(_, ranking)| ranking.iter().copied().find(running))
            .collect();
        let behind = |choice: Option<u32>| -> Vec<String> {
            ballots
                .iter()
                .enumerate()
                .filter(|(ballot, _)| choices[*ballot] == choice)
                .map(|(ballot, (member, _))| {
                    let moved = before.get(ballot).is_some_and(|before| *before != choice);
                    format!("{}{member}", if moved { "+" } else { "" })
                })
                .collect()
        };

        let mut line = format!("**Round {}**", position + 1);
        if !round.eliminated.is_empty() {
            let out: Vec<String> = round.eliminated.iter().map(u32::to_string).collect();
            let _ = write!(line, ", option {} out", out.join(" and "));
        }
        lines.push(line);

        for (number, score) in &round.scores {
            let share = if round.total > 0.0 {
                score / round.total
            } else {
                0.0
            };
            let mut line = format!("Option {number} **{}**", percent(share));
            let members = behind(Some(*number));
            if !members.is_empty() {
                let _ = write!(line, ": {}", members.join(", "));
            }
            lines.push(line);
        }
        if abstained >= 0.0005 && round.total > 0.0 {
            lines.push(format!(
                "Abstained **{}**: {}",
                percent(abstained / round.total),
                abstaining.join(", ")
            ));
        }
        let exhausted = behind(None);
        if !exhausted.is_empty() {
            lines.push(format!("No option left: {}", exhausted.join(", ")));
        }
        before = choices;
    }
    lines
}

/// Abstentions counted in the majority, or 0 if the policy leaves them out
fn abstained_in_majority(tally: &Tally) -> f64 {
    tally
        .rounds
        .first()
        .map_or(0.0, |round| round.total - round.voted())
}

fn percent(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

/// Pack whole lines into chunks of at most `limit` characters
fn chunks(lines: &[String], limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut text = String::new();
    for line in lines {
        // A line longer than a chunk gets cut
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

/// Cut down to at most `limit` characters
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

    /// fyra votes as one, nobara misses its quorum, and a stranger votes too
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
        // fyra gets √2 and dave, alone in the unaffiliated block, gets 1. nobara and ublue miss
        // their quorum, so frank's ballot isn't counted, and neither is mallory's reply since
        // mallory isn't a member
        assert_eq!(
            overview(&closed()),
            "**Option 1 passed**\n\
             Square roots\n\
             Closed <t:1789256856:R>\n\
             ```\n\
             Option 1   ███████░░░░░   58.6%\n\
             Option 2   █████░░░░░░░   41.4%\n\
             Abstained  ░░░░░░░░░░░░    0.0%\n\
             ```\
             Share of the weight of the organisations that reached quorum\n\
             \n\
             Option 1 has **58.6%**, more than half is needed\n\
             Quorum: **2** of **4** organisations, **2** needed\n\
             **3** valid votes, **0** abstained, **2** not counted"
        );
    }

    #[test]
    fn an_open_vote_is_provisional() {
        let tally = count(&[reply("alice", "Option 2"), reply("dave", "abstain")], 2);
        let overview = overview(&tally);
        assert!(overview.starts_with("**Option 2 is ahead**\nEqual shares\nOpen until <t:"));
        assert!(overview.contains("Option 2   ███████░░░░░   58.6%\n"));
        assert!(overview.contains("Abstained  █████░░░░░░░   41.4%\n"));
        // dave's abstention still counts toward the weight
        assert!(overview.contains("Option 2 has **58.6%**, more than half"));
        assert!(
            overview
                .ends_with("**2** needed\n**1** valid vote, **1** abstained, **0** not counted")
        );
    }

    #[test]
    fn a_void_vote_has_no_scores() {
        let tally = count(&[reply("carol", "Option 2")], 48);
        assert_eq!(
            overview(&tally),
            "**Void**, not enough organisations reached quorum\n\
             Closed <t:1789256856:R>\n\
             Quorum: **1** of **4** organisations, **2** needed\n\
             **1** valid vote, **0** abstained, **0** not counted"
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
                "Not counted (1)",
            ]
        );
        // Cards sit side by side, everything else takes the full width
        let inline: Vec<bool> = fields.iter().map(|field| field.2).collect();
        assert_eq!(inline, [true, true, true, true, false]);

        assert_eq!(
            field(&fields, "✅ Fyra Labs"),
            ["2/2 voted, 58.6% of the vote", "**Option 1**: alice, bob"]
        );
        assert_eq!(
            field(&fields, "❌ Nobara"),
            ["1/3 voted, under quorum", "**Option 2**: frank"]
        );
        assert_eq!(field(&fields, "❌ Universal Blue"), ["0/1 voted"]);
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
        let overview = overview(&tally);
        // Only the last round gets bars. Options that didn't get that far say when they went out
        assert!(overview.contains(
            "```\n\
             Option 1   eliminated in round 1\n\
             Option 2   ████████░░░░   70.7%\n\
             Option 3   eliminated in round 1\n\
             Abstained  ████░░░░░░░░   29.3%\n\
             ```\
             Share of the weight of the organisations that reached quorum\n\
             After 2 rounds of instant runoff\n"
        ));

        let fields = fields(&tally);
        let rounds = field(&fields, "Runoff");
        // alice and bob share fyra's √2, dave has 1 and carol's abstention 1
        assert_eq!(
            rounds,
            [
                "**Round 1**, option 1 and 3 out",
                "Option 1 **20.7%**: alice",
                "Option 2 **29.3%**: dave",
                "Option 3 **20.7%**: bob",
                "Abstained **29.3%**: carol",
                "**Round 2**",
                "Option 2 **70.7%**: +alice, +bob, dave",
                "Abstained **29.3%**: carol",
            ]
        );
        assert_eq!(
            field(&fields, "✅ Fyra Labs")[1..],
            ["**Option 1 > 2**: alice", "**Option 3 > 2**: bob"]
        );
    }

    #[test]
    fn abstentions_out_of_the_majority_have_no_share() {
        let policy = Policy {
            abstentions: voting::Abstentions::QuorumOnly,
            ..Policy::default()
        };
        let replies = [
            reply("alice", "Option 1 > Option 2"),
            reply("bob", "Option 3 > Option 2"),
            reply("dave", "Option 2"),
            reply("carol", "Abstain"),
        ];
        let now = sent() + TimeDelta::hours(48);
        let tally = voting::tally(THREE, sent(), &replies, &roster(), &policy, now).unwrap();

        // Bars show each option's share of the total it needs a majority of
        assert!(overview(&tally).contains(
            "```\n\
             Option 1  eliminated in round 1\n\
             Option 2  ████████████  100.0%\n\
             Option 3  eliminated in round 1\n\
             ```\
             Share of the weight of the organisations that reached quorum\n\
             After 2 rounds of instant runoff\n"
        ));
        let fields = fields(&tally);
        assert!(
            !field(&fields, "Runoff")
                .iter()
                .any(|line| line.starts_with("Abstained"))
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
