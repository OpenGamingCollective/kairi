//! One git repository of a mailing list, each commit is a mail

use std::{fs, path::Path, sync::atomic::AtomicBool};

use gix::{ObjectId, Repository, progress::Discard};
use tracing::{debug, warn};

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
    /// Last commit the mirror indexed
    pub indexed: Option<ObjectId>,
}

/// A commit adding or deleting a mail
pub(crate) struct Entry {
    pub commit: ObjectId,
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
            indexed: None,
        })
    }

    pub fn fetch(&self, list_url: &str) -> Result<()> {
        debug!(list = list_url, epoch = self.number, "fetching");
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
        Ok(())
    }

    fn head(&self) -> Result<Option<ObjectId>> {
        Ok(self
            .repo
            .try_find_reference(MASTER)?
            .and_then(|head| head.try_id().map(gix::Id::detach)))
    }

    /// Commits since `indexed`, oldest first, `None` if that one is gone
    pub fn walk(&self) -> Result<Option<Vec<Entry>>> {
        let mut entries = Vec::new();
        let mut next = self.head()?;

        while let Some(id) = next {
            if Some(id) == self.indexed {
                break;
            }
            let commit = self.repo.find_commit(id)?;
            let tree = commit.tree()?;
            let mail = match tree.find_entry("m") {
                Some(entry) => Some((entry.object_id(), false)),
                None => tree.find_entry("d").map(|entry| (entry.object_id(), true)),
            };
            if let Some((blob, deleted)) = mail {
                entries.push(Entry {
                    commit: id,
                    blob,
                    deleted,
                    time: commit.time()?.seconds,
                });
            }
            next = commit.parent_ids().next().map(gix::Id::detach);
        }

        if next.is_none() && self.indexed.is_some() {
            return Ok(None);
        }
        entries.reverse();
        Ok(Some(entries))
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
        let mail = message::index(
            raw,
            list_url,
            self.number,
            entry.blob,
            entry.commit,
            entry.time,
        );
        if mail.is_none() {
            warn!(list = list_url, blob = %entry.blob, "skipping mail without a Message-ID");
        }
        Ok(mail)
    }
}

/// Only we write to the mirror, so any lock left is from a crash
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
    fn walk_returns_new_commits_then_detects_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        let mut epoch = Epoch::open(dir.path(), 0).unwrap();
        epoch.archive(&["a", "b"]);
        epoch.delete("a");

        let entries = epoch.walk().unwrap().unwrap();
        let deleted: Vec<bool> = entries.iter().map(|entry| entry.deleted).collect();
        assert_eq!(deleted, [false, false, true]);

        epoch.indexed = entries.last().map(|entry| entry.commit);
        assert!(epoch.walk().unwrap().unwrap().is_empty());
        epoch.rewrite(&["a"]);
        assert!(epoch.walk().unwrap().is_none());
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
