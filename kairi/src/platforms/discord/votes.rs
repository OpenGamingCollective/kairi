//! `/votes` shows a summary of a mailing list vote, with a button to see who voted what

use std::{collections::BTreeMap, fmt::Write};

use kameo::error::SendError;
use poise::{ChoiceParameter, serenity_prelude as serenity};
use voting::{Outcome, Reason, Round, Standing, Tally, Vote};

use super::{CHOICE, CHOICES, FIELD, FIELDS, SerenityContext, SerenityError, TITLE};
use crate::{
    actors::{Count, CountVotes, FetchList, reader, voting},
    types::MailingListEnum,
};

/// Prefix of the details button's id, which looks like `votes:<list>:<uid>`
const DETAILS: &str = "votes";

struct Field {
    name: String,
    lines: Vec<String>,
    inline: bool,
}

/// Count the votes on a mailing list thread
#[poise::command(slash_command)]
pub async fn votes(
    ctx: SerenityContext<'_>,
    #[description = "Thread of the vote, or the link of any of its mails"]
    #[autocomplete = "threads"]
    thread: String,
    #[description = "List the thread is on, governance by default"] list: Option<MailingListEnum>,
) -> Result<(), SerenityError> {
    ctx.defer().await?;

    let count = CountVotes {
        list: list.unwrap_or(MailingListEnum::Governance),
        thread,
        requested_by: format!("discord /votes @{}", ctx.author().name),
    };
    match count_votes(count).await {
        Ok(count) => {
            let details = serenity::CreateButton::new(details_id(&count))
                .label("Details")
                .style(serenity::ButtonStyle::Secondary);
            let reply = poise::CreateReply::default()
                .embed(summary(&count))
                .components(vec![serenity::CreateActionRow::Buttons(vec![details])]);
            ctx.send(reply).await?;
        }
        Err(error) => {
            ctx.say(error).await?;
        }
    }
    Ok(())
}

/// Reply to the details button
pub async fn on_event(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
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
    let followup = match count_votes(count).await {
        Ok(count) => followup.embed(details(&count)),
        Err(error) => followup.content(error),
    };
    component.create_followup(&ctx.http, followup).await?;
    Ok(())
}

/// Ask the voting actor for a count
async fn count_votes(count: CountVotes) -> Result<Count, String> {
    let voting = voting().map_err(|error| format!("Can't count the votes right now, {error}"))?;
    voting.ask(count).await.map_err(|error| match error {
        SendError::HandlerError(error) => format!("Can't count the votes: {error}"),
        error => format!("Can't count the votes right now: {error}"),
    })
}

fn details_id(count: &Count) -> String {
    format!("{DETAILS}:{}:{}", count.list, count.thread.uid())
}

fn parse_details_id(id: &str) -> Option<(MailingListEnum, String)> {
    let (list, uid) = id
        .strip_prefix(DETAILS)?
        .strip_prefix(':')?
        .split_once(':')?;
    Some((MailingListEnum::from_name(list)?, uid.to_string()))
}

/// Governance threads, newest first
async fn threads(ctx: SerenityContext<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let fetch = FetchList {
        list: MailingListEnum::Governance,
        requested_by: format!("discord /votes autocomplete @{}", ctx.author().name),
    };
    let list = match reader() {
        Ok(reader) => reader.ask(fetch).await.unwrap_or_default(),
        Err(_) => Default::default(),
    };

    let partial = partial.to_lowercase();
    let choices = list
        .threads
        .iter()
        .rev()
        .filter(|thread| thread.name.to_lowercase().contains(&partial))
        .take(CHOICES)
        .map(|thread| {
            let name = format!("{} ({})", thread.name, thread.date.strftime("%Y-%m-%d"));
            serenity::AutocompleteChoice::new(truncate(&name, CHOICE), thread.uid.clone())
        })
        .collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

fn colour(tally: &Tally) -> serenity::Colour {
    match tally.outcome {
        _ if tally.open => serenity::Colour::ORANGE,
        Outcome::Passed(_) => serenity::colours::roles::GREEN,
        Outcome::NoMajority | Outcome::Tied | Outcome::Void => serenity::Colour::RED,
    }
}

/// Vote result summary embed
fn summary(count: &Count) -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title(truncate(count.thread.name(), TITLE))
        .url(count.thread.url())
        .description(overview(&count.tally))
        .colour(colour(&count.tally))
}

/// Vote details and breakdown embed
fn details(count: &Count) -> serenity::CreateEmbed {
    let tally = &count.tally;
    let title = truncate(count.thread.name(), TITLE);
    let footer = format!(
        "{} out of {} members eligible for voting",
        count.roster.eligible_count(),
        count.roster.member_count()
    );

    // Long fields spill over into the next one as long as the embed has room
    let size = |text: &str| text.chars().count();
    let mut room = serenity::EMBED_MAX_LENGTH.saturating_sub(size(&title) + size(&footer));
    let fields: Vec<(String, String, bool)> = fields(tally)
        .into_iter()
        .flat_map(
            |Field {
                 name,
                 lines,
                 inline,
             }| {
                let texts = chunks(&lines, FIELD).into_iter().enumerate();
                texts.map(move |(position, text)| match position {
                    0 => (name.clone(), text, inline),
                    _ => (format!("{name}, continued"), text, inline),
                })
            },
        )
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
        .url(count.thread.url())
        .fields(fields)
        .footer(serenity::CreateEmbedFooter::new(footer))
        .colour(colour(tally))
}

fn overview(tally: &Tally) -> String {
    let mut text = headline(tally);

    if let Some(round) = tally.rounds.last() {
        let _ = write!(text, "\n```\n{}\n```", chart(tally, round));
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
            && tally.total > 0.0
        {
            let share = percent(tally.share(*score));
            let _ = writeln!(
                text,
                "Option {number} has **{share}**, more than half is needed"
            );
        }
    } else {
        text.push('\n');
    }

    let entities = tally.standings.len();
    let needed = tally.needed;
    // For auditing: counted ballots, abstentions, then the rest (under quorum or rejected)
    let abstained = tally
        .ballots()
        .filter(|ballot| ballot.vote == Vote::Abstain)
        .count();
    let valid = tally.ballots().count() - abstained;
    let left_out = tally.cast() - valid - abstained + tally.rejected.len();
    let _ = write!(
        text,
        "Quorum: **{}** of **{entities}** organisations, **{needed}** needed\n\
         **{valid}** valid {}, **{abstained}** abstained, **{left_out}** not counted",
        tally.counting(),
        if valid == 1 { "vote" } else { "votes" },
    );
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
        Outcome::NoMajority if tally.open => "**No majority** so far".to_string(),
        Outcome::NoMajority => {
            "**No majority**, no option has more than half of the vote".to_string()
        }
        Outcome::Tied if tally.open => "**Tied** so far".to_string(),
        Outcome::Tied => "**Tied**, the vote has to be held again".to_string(),
        Outcome::Void if tally.open => "**No quorum** yet".to_string(),
        Outcome::Void => "**Void**, not enough organisations reached quorum".to_string(),
    };

    let closes = tally.motion.closes.as_second();
    let window = if tally.open {
        format!("Open until <t:{closes}:f>, <t:{closes}:R>, the count can still change")
    } else {
        format!("Closed <t:{closes}:R>")
    };
    format!("{outcome}\n{window}")
}

/// One bar per row
fn chart(tally: &Tally, round: &Round) -> String {
    const BAR: usize = 12;
    let bar = |share: f64| {
        let filled = ((share * BAR as f64).round() as usize).min(BAR);
        let bar = "█".repeat(filled) + &"░".repeat(BAR - filled);
        format!("{bar}  {:>6}", percent(share))
    };

    let mut rows: Vec<(String, String)> = tally
        .motion
        .options
        .iter()
        .filter_map(|option| {
            let score = round
                .scores
                .iter()
                .find(|(number, _)| *number == option.number);
            let row = match score {
                Some((_, score)) => bar(tally.share(*score)),
                None => {
                    let out = |round: &Round| round.eliminated.contains(&option.number);
                    let round = tally.rounds.iter().position(out)? + 1;
                    format!("eliminated in round {round}")
                }
            };
            Some((format!("Option {}", option.number), row))
        })
        .collect();
    let exhausted = tally.share(tally.total - round.continuing_weight());
    if exhausted >= 0.0005 {
        rows.push(("No option left".to_string(), bar(exhausted)));
    }

    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    let lines: Vec<String> = rows
        .iter()
        .map(|(label, row)| format!("{label:<width$}  {row}"))
        .collect();
    lines.join("\n")
}

fn fields(tally: &Tally) -> Vec<Field> {
    let mut fields: Vec<Field> = Vec::new();

    if tally.rounds.len() > 1 {
        fields.push(Field {
            name: "Runoff".to_string(),
            lines: rounds(tally),
            inline: false,
        });
    }

    fields.extend(tally.standings.iter().map(card));

    if !tally.rejected.is_empty() {
        let name = format!("Not counted ({})", tally.rejected.len());
        let rejected = tally.rejected.iter().map(|rejected| {
            let reason = match rejected.reason {
                Reason::UnknownSender => "voted from an address that isn't on the roster",
                Reason::OutsideWindow => "voted while the vote wasn't open",
                Reason::Invalid => "replied without a valid vote",
            };
            format!("[{}]({}) {reason}", rejected.author, rejected.url)
        });
        fields.push(Field {
            name,
            lines: rejected.collect(),
            inline: false,
        });
    }
    fields
}

/// An organisation's turnout, weight and votes
fn card(standing: &Standing) -> Field {
    let mark = if standing.passed_quorum { "✅" } else { "❌" };
    let turnout = format!(
        "{}/{} voted",
        standing.ballots.len(),
        standing.entity.members.len()
    );
    let mut lines = vec![if standing.passed_quorum {
        format!("{turnout}, {} of the vote", percent(standing.weight))
    } else if !standing.ballots.is_empty() {
        format!("{turnout}, under quorum")
    } else {
        turnout
    }];

    // Same order as the motion, with abstentions last
    let mut votes: BTreeMap<&Vote, Vec<&str>> = BTreeMap::new();
    for ballot in &standing.ballots {
        votes.entry(&ballot.vote).or_default().push(&ballot.member);
    }

    for (vote, members) in votes {
        let label = match vote {
            Vote::Abstain => "Abstained".to_string(),
            Vote::Ranked(options) => {
                let options: Vec<String> = options.iter().map(u32::to_string).collect();
                format!("Option {}", options.join(" > "))
            }
        };
        lines.push(format!("**{label}**: {}", members.join(", ")));
    }
    Field {
        name: format!("{mark} {}", standing.entity.name),
        lines,
        inline: true,
    }
}

/// Every runoff round
fn rounds(tally: &Tally) -> Vec<String> {
    let mut lines = Vec::new();
    // Which option each ballot backed in the previous round
    let mut before: Option<&Round> = None;
    for (position, round) in tally.rounds.iter().enumerate() {
        let behind = |choice: Option<u32>| -> Vec<String> {
            round
                .choices
                .iter()
                .enumerate()
                .filter(|(_, (_, backs))| *backs == choice)
                .map(|(ballot, (member, _))| {
                    let moved = before.is_some_and(|before| before.choices[ballot].1 != choice);
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
            let mut line = format!("Option {number} **{}**", percent(tally.share(*score)));
            let members = behind(Some(*number));
            if !members.is_empty() {
                let _ = write!(line, ": {}", members.join(", "));
            }
            lines.push(line);
        }
        let exhausted = behind(None);
        if !exhausted.is_empty() {
            lines.push(format!("No option left: {}", exhausted.join(", ")));
        }
        before = Some(round);
    }
    lines
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
    use jiff::{SignedDuration, Timestamp};
    use voting::{Policy, Reply, Roster};

    use super::*;

    const MAIL: &str = "Option 1: Square roots\nOption 2: Equal shares\nVOTE BELOW THIS LINE\n";
    const THREE: &str = "Option 1: a\nOption 2: b\nOption 3: c\n";

    fn sent() -> Timestamp {
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
            date: sent() + SignedDuration::from_hours(1),
            url: format!("https://l/{author}/"),
            body: format!("VOTE BELOW THIS LINE\n{body}"),
        }
    }

    fn count_of(mail: &str, replies: &[Reply], hours: i64) -> Tally {
        let now = sent() + SignedDuration::from_hours(hours);
        voting::tally(mail, sent(), replies, &roster(), &Policy::default(), now).unwrap()
    }

    fn count(replies: &[Reply], hours: i64) -> Tally {
        count_of(MAIL, replies, hours)
    }

    fn field<'a>(fields: &'a [Field], name: &str) -> &'a [String] {
        &fields
            .iter()
            .find(|field| field.name == name)
            .unwrap()
            .lines
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
             Option 1  ███████░░░░░   58.6%\n\
             Option 2  █████░░░░░░░   41.4%\n\
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
        assert!(overview.contains("Option 2  ████████████  100.0%\n"));
        assert!(overview.contains("Option 2 has **100.0%**, more than half"));
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
        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
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
        let inline: Vec<bool> = fields.iter().map(|field| field.inline).collect();
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
             Option 1  eliminated in round 1\n\
             Option 2  ████████████  100.0%\n\
             Option 3  eliminated in round 1\n\
             ```\
             Share of the weight of the organisations that reached quorum\n\
             After 2 rounds of instant runoff\n"
        ));

        let fields = fields(&tally);
        let rounds = field(&fields, "Runoff");
        // alice and bob share fyra's √2, dave and carol the unaffiliated √2
        assert_eq!(
            rounds,
            [
                "**Round 1**, option 1 and 3 out",
                "Option 1 **29.3%**: alice",
                "Option 2 **41.4%**: dave",
                "Option 3 **29.3%**: bob",
                "**Round 2**",
                "Option 2 **100.0%**: +alice, +bob, dave",
            ]
        );
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
