//! One git repository of a mailing list, each commit is a mail

use std::{fs, io, path::Path, sync::atomic::AtomicBool, time::Instant};

use gix::{ObjectId, Repository, progress::Discard};
use tracing::{debug, info, instrument, warn};

use crate::{
    errors::Result,
    message::{self, Mail},
};

/// The only branch public-inbox writes
const MASTER: &str = "refs/heads/master";

#[derive(Debug, Clone)]
pub(crate) struct Epoch {
    pub number: u32,
    repo: Repository,
}

/// A commit adding or deleting a mail
pub(crate) struct Entry {
    blob: ObjectId,
    pub deleted: bool,
    time: i64,
}

impl Epoch {
    pub fn open(dir: &Path, number: u32) -> Result<Self> {
        let path = dir.join(format!("git/{number}.git"));

        if path.exists() {
            remove_stale_locks(&path)?;
        } else {
            debug!(epoch = number, "creating the repository");
            // Created aside then renamed, a crash never leaves a broken repository
            let tmp = path.with_extension("tmp");
            if tmp.exists() {
                fs::remove_dir_all(&tmp)?;
            }
            fs::create_dir_all(&tmp)?;
            gix::init_bare(&tmp)?;
            fs::rename(&tmp, &path)?;
        }

        Ok(Self {
            number,
            repo: gix::open_opts(path, gix::open::Options::isolated())?,
        })
    }

    /// Open without creating it nor touching its locks, `None` if it isn't mirrored
    pub fn open_readonly(dir: &Path, number: u32) -> Result<Option<Self>> {
        let path = dir.join(format!("git/{number}.git"));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self {
            number,
            repo: gix::open_opts(path, gix::open::Options::isolated())?,
        }))
    }

    /// Numbers of the epochs mirrored in `dir`, oldest first
    pub fn mirrored(dir: &Path) -> Result<Vec<u32>> {
        let entries = match fs::read_dir(dir.join("git")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut numbers = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            // `<n>.tmp` is a creation that never finished
            if let Some(number) = name
                .to_str()
                .and_then(|name| name.strip_suffix(".git"))
                .and_then(|number| number.parse().ok())
            {
                numbers.push(number);
            }
        }
        numbers.sort_unstable();
        Ok(numbers)
    }

    #[instrument(level = "debug", skip_all, fields(epoch = self.number))]
    pub fn fetch(&self, list_url: &str) -> Result<()> {
        let before = self.head()?;
        if before.is_none() {
            info!("cloning");
        } else {
            debug!("fetching");
        }
        let started = Instant::now();

        self.repo
            .remote_at(format!("{list_url}git/{}.git", self.number))?
            // Forced, purging a mail rewrites history
            .with_refspecs(
                [format!("+{MASTER}:{MASTER}").as_str()],
                gix::remote::Direction::Fetch,
            )?
            .connect(gix::remote::Direction::Fetch)?
            .prepare_fetch(Discard, Default::default())?
            .receive(Discard, &AtomicBool::new(false))?;

        let after = self.head()?;
        let elapsed_ms = started.elapsed().as_millis();
        match (before, after) {
            (None, _) => info!(elapsed_ms, "cloned"),
            (Some(before), Some(after)) if before != after => {
                info!(%before, %after, elapsed_ms, "fetched new commits");
            }
            _ => debug!(elapsed_ms, "already up to date"),
        }
        Ok(())
    }

    /// Tip of the epoch, `None` before its first fetch
    pub fn head(&self) -> Result<Option<ObjectId>> {
        Ok(self
            .repo
            .try_find_reference(MASTER)?
            .and_then(|head| head.try_id().map(gix::Id::detach)))
    }

    /// Every commit, oldest first, purges included
    pub fn walk(&self) -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        let mut next = self.head()?;

        while let Some(id) = next {
            let commit = self.repo.find_commit(id)?;
            let tree = commit.tree()?;
            let mail = match tree.find_entry("m") {
                Some(entry) => Some((entry.object_id(), false)),
                None => tree.find_entry("d").map(|entry| (entry.object_id(), true)),
            };
            if let Some((blob, deleted)) = mail {
                entries.push(Entry {
                    blob,
                    deleted,
                    time: commit.time()?.seconds,
                });
            }
            next = commit.parent_ids().next().map(gix::Id::detach);
        }

        entries.reverse();
        Ok(entries)
    }

    pub fn blob(&self, id: ObjectId) -> Result<Vec<u8>> {
        Ok(self.repo.find_blob(id)?.detach().data)
    }

    pub fn mail(&self, entry: &Entry, list_url: &str) -> Result<Option<Mail>> {
        let raw = &self.repo.find_blob(entry.blob)?.data;
        // Purged mails are left as empty blobs
        if raw.is_empty() {
            return Ok(None);
        }
        let mail = message::index(raw, list_url, self.number, entry.blob, entry.time);
        if mail.is_none() {
            warn!(list = list_url, blob = %entry.blob, "skipping mail without a Message-ID");
        }
        Ok(mail)
    }
}

/// Only the mirror writes, the reader opens read-only, so any lock left is from a crash
fn remove_stale_locks(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            // objects/ is large and gix writes packs atomically
            if !path.ends_with("objects") {
                remove_stale_locks(&path)?;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "lock")
        {
            warn!(path = %path.display(), "removing stale lock");
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Write mails like public-inbox does
#[cfg(test)]
impl Epoch {
    /// Commit a mail as the only file of the tree, on top of `parent`
    pub fn commit(&self, parent: Option<ObjectId>, file: &str, id: &str) -> ObjectId {
        use gix::{
            objs::{Commit, Tree, tree},
            refs::transaction::PreviousValue,
        };

        let raw = format!("Subject: {id}\r\nMessage-ID: <{id}>\r\n\r\nbody of {id}");
        let tree = Tree {
            entries: vec![tree::Entry {
                mode: tree::EntryKind::Blob.into(),
                filename: file.into(),
                oid: self.repo.write_blob(raw).unwrap().detach(),
            }],
        };
        let signature = gix::actor::Signature::default();
        let commit = Commit {
            tree: self.repo.write_object(&tree).unwrap().detach(),
            parents: parent.into_iter().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: "mail".into(),
            extra_headers: Vec::new(),
        };

        let id = self.repo.write_object(&commit).unwrap().detach();
        self.repo
            .reference(MASTER, id, PreviousValue::Any, "test")
            .unwrap();
        id
    }

    pub fn archive(&self, ids: &[&str]) {
        for id in ids {
            self.commit(self.head().unwrap(), "m", id);
        }
    }

    pub fn delete(&self, id: &str) {
        self.commit(self.head().unwrap(), "d", id);
    }

    /// Replace the history, like a purge does
    pub fn rewrite(&self, ids: &[&str]) {
        let mut parent = None;
        for id in ids {
            parent = Some(self.commit(parent, "m", id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_returns_every_commit_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let epoch = Epoch::open(dir.path(), 0).unwrap();
        epoch.archive(&["a", "b"]);
        epoch.delete("a");

        let entries = epoch.walk().unwrap();
        let deleted: Vec<bool> = entries.iter().map(|entry| entry.deleted).collect();
        assert_eq!(deleted, [false, false, true]);

        epoch.rewrite(&["a"]);
        let entries = epoch.walk().unwrap();
        let deleted: Vec<bool> = entries.iter().map(|entry| entry.deleted).collect();
        assert_eq!(deleted, [false]);
    }

    #[test]
    fn stale_lock_is_removed_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Epoch::open(dir.path(), 0)
            .unwrap()
            .repo
            .path()
            .to_path_buf();
        let lock = repo.join("refs/heads/master.lock");
        fs::write(&lock, "").unwrap();

        let epoch = Epoch::open(dir.path(), 0).unwrap();
        assert!(!lock.exists());
        // The repository is writable again
        epoch.archive(&["a"]);
    }

    #[test]
    fn interrupted_creation_is_redone() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("git/0.tmp/junk")).unwrap();

        Epoch::open(dir.path(), 0).unwrap().archive(&["a"]);
        assert!(!dir.path().join("git/0.tmp").exists());
    }
}
