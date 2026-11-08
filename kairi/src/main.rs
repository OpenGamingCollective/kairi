use std::{fs, path::Path};

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    println!("hello world!");

    let file_content = fs::read_to_string(Path::new("./config.toml"))?;

    let config = kairi_config::Config::build(&file_content)?;

    let client = typesafe_api::TypeSafeClient::build(
        &config.typesafe.account_id,
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
