//! Handle scraping a mailing list

use std::str::FromStr;

use reqwest::{Client, StatusCode};
use scraper::{Html, Selector};
use tokio::sync::RwLock;
use url::Url;

use crate::{
    errors::{MailingListError, Result},
    message::{fetch_messages, mbox_url},
    thread::Thread,
};

/// Represent a mailing list
#[derive(Debug)]
pub struct MailingList {
    url: Url,
    client: Client,
    name: String,
    description: String,
    threads: RwLock<Vec<Thread>>,
}

impl MailingList {
    pub async fn build(url: &str, client_name: &str) -> Result<Self> {
        let mut url = Url::from_str(url)?;

        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }

        let res = reqwest::get(url.clone()).await?;

        if res.status() != StatusCode::OK {
            return Err(MailingListError::UrlStatusCodeError(
                url.to_string(),
                res.status(),
            ));
        }

        let html = res.text().await?;

        let name = url
            .path_segments()
            .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();
        let description = list_description(&html);

        let client = Client::builder().user_agent(client_name).build()?;

        Ok(Self {
            url,
            client,
            name,
            description,
            threads: RwLock::default(),
        })
    }

    pub fn url(&self) -> Url {
        self.url.clone()
    }

    /// HTTP client used by this list, shared by callers to fetch thread data
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Name of the mailing list, taken from the last URL path segment
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Description of the mailing list, taken from the list page title
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the threads
    pub async fn threads(&self) -> Vec<Thread> {
        self.threads.read().await.clone()
    }

    /// Get a thread using its uid
    pub async fn get_thread(&self, uid: &str) -> Result<Thread> {
        if uid.is_empty() {
            return Err(MailingListError::ThreadUidEmpty);
        }

        // Scan the threads if vec is empty
        {
            let lock = self.threads.read().await;
            if lock.is_empty() {
                drop(lock);
                self.get_all_threads().await?;
            }
        }

        let lock = self.threads.read().await;

        lock.iter()
            .find(|t| t.uid() == uid)
            .cloned()
            .ok_or_else(|| MailingListError::ThreadNotFound(uid.to_string()))
    }

    /// Fetch all threads of a mailing list
    // TODO: handle multiple pages
    pub async fn get_all_threads(&self) -> Result<Vec<Thread>> {
        let html = self
            .client
            .get(self.url.clone())
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let urls = thread_urls(&html, &self.url)?;

        let mut threads: Vec<Thread> = Vec::new();

        for url in urls {
            let messages = match fetch_messages(&self.client, &mbox_url(&url)?).await {
                Ok(messages) => messages,
                Err(error) => {
                    eprintln!("skipping thread {url}: {error}");
                    continue;
                }
            };

            threads.push(Thread::from_url(url.as_str(), &messages)?);
        }

        let mut lock = self.threads.write().await;

        *lock = threads.clone();

        Ok(threads)
    }
}

/// List description scraped from a list page's `<title>`
fn list_description(html: &str) -> String {
    let document = Html::parse_document(html);

    let title_selector = Selector::parse("title").unwrap();

    document
        .select(&title_selector)
        .next()
        .map(|title| title.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

/// Thread URLs scraped from a list page
fn thread_urls(html: &str, base: &Url) -> Result<Vec<Url>> {
    let document = Html::parse_document(html);

    let thread_selector = Selector::parse(r#"pre > a[href*="/T/"]"#).unwrap();

    let mut urls: Vec<Url> = Vec::new();

    for link in document.select(&thread_selector) {
        let Some(sibling) = link.next_sibling() else {
            continue;
        };

        let Some(text) = sibling.value().as_text() else {
            continue;
        };

        let metadata = text.trim_start().lines().next().unwrap_or("");
        if !metadata.contains(" UTC") {
            continue;
        }
        let href = link.value().attr("href").unwrap();
        urls.push(base.join(href)?);
    }

    Ok(urls)
}
