//! The manifest listing the epochs of a mailing list

use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
};

use flate2::read::GzDecoder;
use reqwest::Client;
use serde::Deserialize;

use crate::errors::Result;

#[derive(Deserialize)]
struct Entry {
    description: Option<String>,
}

/// Description and epochs of the list at `url`
pub(crate) async fn fetch(client: &Client, url: &str) -> Result<(String, Vec<u32>)> {
    let bytes = client
        .get(format!("{url}manifest.js.gz"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let mut json = Vec::new();
    GzDecoder::new(bytes.as_ref()).read_to_end(&mut json)?;
    parse(&json)
}

/// Keys are `/<list>/git/<epoch>.git`, descriptions `<description> [epoch <epoch>]`
fn parse(json: &[u8]) -> Result<(String, Vec<u32>)> {
    let entries: HashMap<String, Entry> = serde_json::from_slice(json)?;
    let epochs: BTreeMap<u32, Option<String>> = entries
        .into_iter()
        .filter_map(|(path, entry)| {
            let epoch = path.rsplit_once("/git/")?.1.strip_suffix(".git")?;
            Some((epoch.parse().ok()?, entry.description))
        })
        .collect();

    // A list nobody posted to yet has no epoch, so no description either
    let description = epochs
        .last_key_value()
        .and_then(|(_, description)| description.as_deref())
        .unwrap_or_default();
    let description = description
        .rsplit_once(" [epoch ")
        .map_or(description, |(description, _)| description)
        .to_string();

    Ok((description, epochs.into_keys().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::MailingListError;

    #[test]
    fn manifest_lists_the_epochs() {
        let manifest = parse(
            br#"{"/list/git/1.git":{"description":"A list [epoch 1]"},
                "/list/git/0.git":{"description":"A list [epoch 0]"}}"#,
        );

        assert_eq!(manifest.unwrap(), ("A list".to_string(), vec![0, 1]));
        assert_eq!(parse(b"{}").unwrap(), (String::new(), vec![]));
        assert!(matches!(parse(b"<html>"), Err(MailingListError::Json(_))));
    }
}
