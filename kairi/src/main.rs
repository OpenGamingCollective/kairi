use std::path::Path;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use tokio::fs;
use tracing::{Level, error, info};
use tracing_subscriber::FmtSubscriber;

use crate::args::Args;

mod args;

#[tokio::main]
async fn main() -> Result<()> {
    let arg = Args::parse();

    let log_level = if arg.debug { Level::TRACE } else { Level::INFO };

    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;

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
                println!("config parsed correctly");
                Ok(())
            }
            Err(err) => Err(anyhow!(err)),
        };
    }

    let config = kairi_config::Config::build(&file_content)?;

    let client = typesafe_api::TypeSafeClient::build(
        "https://api.typesafe.ai/v1/systemone",
        &config.typesafe.token,
    );

    let request = typesafe_api::TypeSafeRequestBuilder::new()
        .model("jev-latest")
        .state("Question about Linux")
        .noul("language", "is Linux built in Go?", None)
        .build();

    let res = request.send(client).await;

    match res {
        Ok(response) => {
            info!("typesafe response: {:#?}", response)
        }
        Err(err) => {
            error!("typesafe error: {:#?}", err)
        }
    }

    Ok(())
}
