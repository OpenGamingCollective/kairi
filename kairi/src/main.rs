use std::{future::pending, path::Path};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use kameo::actor::Spawn;
use tokio::fs;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

use crate::{
    actors::{Kairi, KairiArg},
    args::Args,
};

mod actors;
mod args;
mod platforms;
mod types;

/// kairi/0.1.0
pub(crate) const USER_AGENT: &str = concat!(
    "kairi/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/OpenGamingCollective/kairi)"
);

#[tokio::main]
async fn main() -> Result<()> {
    let arg = Args::parse();

    // DEBUG
    let log_level = if arg.debug { Level::TRACE } else { Level::INFO };

    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;

    let _console = if arg.debug {
        Some(kameo::console::serve("127.0.0.1:9999").await?)
    } else {
        None
    };

    let file_content = match arg.config {
        Some(path) => {
            info!("config file specified, using {path}");
            fs::read_to_string(Path::new(&path)).await
        }
        None => {
            info!("no config file specified, using default /etc/kairi/config.toml");
            fs::read_to_string(Path::new("/etc/kairi/config.toml")).await
        }
    }?;
    // Return the parsing result
    if arg.test {
        return match kairi_config::Config::build(&file_content) {
            Ok(_) => {
                info!("config parsed correctly");
                Ok(())
            }
            Err(err) => Err(anyhow!(err)),
        };
    }

    let config = kairi_config::Config::build(&file_content)?;

    let _kairi_supervisor = Kairi::spawn(KairiArg { config });
    info!("Kairi started successfully");
    pending::<()>().await;

    Ok(())
}
