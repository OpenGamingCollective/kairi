use std::sync::Arc;

use poise::serenity_prelude as serenity;
use tokio::sync::Mutex;
use voting::Policy;

mod votes;

pub struct Data {
    pub policy: Policy,
}

type SerenityError = Box<dyn std::error::Error + Send + Sync>;

type SerenityContext<'a> = poise::Context<'a, Data, SerenityError>;

/// Discord's limits
const TITLE: usize = 256;
const FIELD: usize = 1024;
const FIELDS: usize = 25;
const CHOICE: usize = 100;
const CHOICES: usize = 25;

pub struct DiscordBot {
    pub client: Arc<Mutex<serenity::Client>>,
    pub http: Arc<serenity::Http>,
}

impl DiscordBot {
    pub async fn build(token: &str, data: Data) -> anyhow::Result<Self> {
        let intents = serenity::GatewayIntents::all();

        let framework = poise::Framework::<Data, SerenityError>::builder()
            .options(poise::FrameworkOptions {
                commands: vec![ping(), votes::votes()],
                event_handler: |framework, event| {
                    Box::pin(votes::on_event(framework.serenity_context, event))
                },
                ..Default::default()
            })
            .setup(move |ctx, _ready, framework| {
                Box::pin(async move {
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                    Ok(data)
                })
            })
            .build();

        let client = serenity::ClientBuilder::new(token, intents)
            .framework(framework)
            .await?;

        let http: Arc<serenity::Http> = client.http.clone();

        let client: Arc<Mutex<serenity::prelude::Client>> = Arc::new(Mutex::new(client));

        Ok(Self { client, http })
    }
}

#[poise::command(slash_command, prefix_command)]
async fn ping(ctx: SerenityContext<'_>) -> Result<(), SerenityError> {
    ctx.say("Pong !!").await?;
    Ok(())
}
