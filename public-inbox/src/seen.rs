//! Cursor of the last mail the caller handled, kept in `<dir>/seen`

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

use gix::ObjectId;

use crate::{errors::Result, message::Mail};

#[derive(Debug)]
struct Cursor {
    commit: Option<ObjectId>,
    /// Empty when nothing was handled yet
    message_id: String,
}

#[derive(Debug, Default)]
pub(crate) struct Seen {
    path: PathBuf,
    /// `None` until the first start completes
    cursor: Option<Cursor>,
    /// Indexing from scratch, the mails up to the cursor were handled
    catching_up: bool,
    /// The cursor's mail was met while catching up
    found: bool,
    /// Its exact commit was, so later copies of its Message-ID are new mails
    found_commit: bool,
}

impl Seen {
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join("seen");
        let cursor = match fs::read_to_string(&path) {
            Ok(content) => {
                let content = content.trim_end();
                // Stored as `<commit> <Message-ID>`, without a valid commit it's all a Message-ID
                let (commit, message_id) = match content.split_once(' ') {
                    Some((commit, id))
                        if let Ok(commit) = ObjectId::from_hex(commit.as_bytes()) =>
                    {
                        (Some(commit), id)
                    }
                    _ => (None, content),
                };
                Some(Cursor {
                    commit,
                    message_id: message_id.to_string(),
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };

        let mut seen = Self {
            path,
            cursor,
            ..Self::default()
        };
        seen.rewind();
        Ok(seen)
    }

    /// Whether no start completed yet
    pub fn is_new(&self) -> bool {
        self.cursor.is_none()
    }

    /// Look for the cursor while indexing from scratch
    pub fn rewind(&mut self) {
        self.catching_up = true;
        self.found = self
            .cursor
            .as_ref()
            .is_none_or(|cursor| cursor.message_id.is_empty());
        self.found_commit = false;
    }

    /// While catching up, whether `mail` is the cursor's commit, or a copy of its
    /// Message-ID as long as that commit was not found
    pub fn reached(&mut self, mail: &Mail) -> bool {
        let Some(cursor) = self.cursor.as_ref().filter(|_| self.catching_up) else {
            return false;
        };
        let commit = cursor.commit == Some(mail.commit);
        let reached = commit || !self.found_commit && cursor.message_id == mail.message_id;
        self.found_commit |= commit;
        self.found |= reached;
        reached
    }

    /// Stop catching up, `true` if the cursor's mail was purged
    pub fn caught_up(&mut self) -> bool {
        let lost = self.catching_up && !self.found;
        self.catching_up = false;
        lost
    }

    /// Synced then renamed, a crash never leaves a torn or empty cursor
    pub fn write(&mut self, mail: Option<&Mail>) -> Result<()> {
        let tmp = self.path.with_extension("tmp");
        let mut file = fs::File::create(&tmp)?;
        if let Some(mail) = mail {
            write!(file, "{} {}", mail.commit, mail.message_id)?;
        }
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;

        self.cursor = Some(Cursor {
            commit: mail.map(|mail| mail.commit),
            message_id: mail.map(|mail| mail.message_id.clone()).unwrap_or_default(),
        });
        Ok(())
    }
}
