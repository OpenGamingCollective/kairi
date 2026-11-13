//! Index of a mirrored list, written by the mirror after each fetch, read by the reader
//!
//! ```text
//! <dir>/index -> index-<stamp>/   swapped atomically
//! <dir>/index-<stamp>/list.json   heads, description and thread summaries
//! <dir>/index-<stamp>/threads/<uid>.json
//! ```

use std::{
    fs, io,
    os::unix::fs::symlink,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use gix::ObjectId;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::{debug, info, instrument, warn};

use crate::{
    epoch::Epoch,
    errors::Result,
    reader::List,
    thread::{Thread, ThreadSummary, Threads},
};

const INDEX: &str = "index";
const LIST: &str = "list.json";
const THREADS: &str = "threads";

/// Tip of each epoch the index was built from
type Heads = Vec<(u32, Option<ObjectId>)>;

#[derive(Serialize, Deserialize)]
struct ListFile {
    heads: Heads,
    #[serde(flatten)]
    list: List,
}

/// Rebuild the index if the git or the description changed since the last one, `true` if it did
///
/// Nothing is fsynced, a power loss can leave empty files behind the swapped symlink.
/// `verify` reads every file of the index too, rebuilding it if one is broken
#[instrument(level = "debug", skip_all, fields(verify = verify))]
pub(crate) fn reindex(dir: &Path, url: &str, description: &str, verify: bool) -> Result<bool> {
    let epochs = Epoch::mirrored(dir)?
        .into_iter()
        .filter_map(|number| Epoch::open_readonly(dir, number).transpose())
        .collect::<Result<Vec<_>>>()?;
    let heads = epochs
        .iter()
        .map(|epoch| Ok((epoch.number, epoch.head()?)))
        .collect::<Result<Heads>>()?;

    let reason = match read::<ListFile>(&dir.join(INDEX).join(LIST)) {
        Err(error) => {
            warn!(%error, "the index's list is broken");
            "broken"
        }
        Ok(None) => "missing",
        Ok(Some(indexed)) if indexed.heads != heads => "the git changed",
        Ok(Some(indexed)) if indexed.list.description != description => "the description changed",
        Ok(Some(indexed)) if verify && !threads_intact(dir, &indexed.list) => "broken",
        Ok(Some(indexed)) => {
            debug!(
                verified = verify,
                threads = indexed.list.threads.len(),
                "index up to date"
            );
            return Ok(false);
        }
    };
    info!(reason, "rebuilding the index");

    let started = Instant::now();
    let threads = write(dir, heads, description, &build(&epochs, url)?)?;
    info!(
        threads,
        elapsed_ms = started.elapsed().as_millis(),
        "index written"
    );
    Ok(true)
}

/// Whether every listed thread has a file that parses, the first broken one is logged
fn threads_intact(dir: &Path, list: &List) -> bool {
    list.threads
        .iter()
        .all(|summary| match read_thread(dir, &summary.uid) {
            Ok(Some(_)) => true,
            Ok(None) => {
                warn!(uid = summary.uid, "an indexed thread's file is missing");
                false
            }
            Err(error) => {
                warn!(uid = summary.uid, %error, "an indexed thread's file is broken");
                false
            }
        })
}

/// Thread every mail of the epochs, the git is walked from scratch
fn build(epochs: &[Epoch], url: &str) -> Result<Threads> {
    let mut threads = Threads::default();
    for epoch in epochs {
        for entry in epoch.walk()? {
            let Some(mail) = epoch.mail(&entry, url)? else {
                continue;
            };
            if entry.deleted {
                threads.remove(&mail.message_id);
            } else {
                threads.insert(mail);
            }
        }
    }
    Ok(threads)
}

/// Written aside then swapped in, the reader never sees half an index, returns the thread count
fn write(dir: &Path, heads: Heads, description: &str, threads: &Threads) -> Result<usize> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{INDEX}-{stamp}");
    let tmp = dir.join(format!("{name}.tmp"));

    let threads = threads.all();
    fs::create_dir_all(tmp.join(THREADS))?;
    for thread in &threads {
        let path = tmp.join(THREADS).join(format!("{}.json", thread.uid()));
        fs::write(path, serde_json::to_vec(thread)?)?;
    }
    let list = ListFile {
        heads,
        list: List {
            description: description.to_string(),
            threads: threads.iter().map(ThreadSummary::from).collect(),
        },
    };
    fs::write(tmp.join(LIST), serde_json::to_vec(&list)?)?;
    fs::rename(&tmp, dir.join(&name))?;

    // Renaming a symlink over the old one is atomic, unlike replacing a directory
    let link = dir.join(format!("{INDEX}.tmp"));
    remove(&link)?;
    symlink(&name, &link)?;
    fs::rename(&link, dir.join(INDEX))?;

    // The previous index, and any left by a crash
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with(&format!("{INDEX}-")) && file_name != name {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(threads.len())
}

/// The indexed list, `None` before the first index
pub(crate) fn read_list(dir: &Path) -> Result<Option<List>> {
    Ok(read::<ListFile>(&dir.join(INDEX).join(LIST))?.map(|file| file.list))
}

/// An indexed thread by its uid, `None` if there's no such thread
pub(crate) fn read_thread(dir: &Path, uid: &str) -> Result<Option<Thread>> {
    // Comes from users, never let it walk out of the index
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(None);
    }
    read(&dir.join(INDEX).join(THREADS).join(format!("{uid}.json")))
}

fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(json) => Ok(Some(serde_json::from_slice(&json)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}
