use std::{fs, path::Path};

use anyhow::{Result, anyhow};
use clap::Parser;

use crate::args::Args;

mod args;

#[tokio::main]
async fn main() -> Result<()> {
    let arg = Args::parse();

    let file_content = match arg.config {
        Some(path) => {
            println!("config file specified, using {path}");
            fs::read_to_string(Path::new(&path))
        }
        None => {
            println!("no config file specified, using default /etc/kairi/config.toml");
            fs::read_to_string(Path::new("/etc/kairi/config.toml"))
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

    println!("typesafe endpoint: {:?}", request.send(client).await?);

    Ok(())
}
