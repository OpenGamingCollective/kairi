//! Local copy of a mailing list: its epochs, threads and pending mails

use std::path::PathBuf;

use mail_parser::Message;

use crate::{
    epoch::Epoch,
    errors::{MailingListError, Result},
    message::{self, Mail},
    seen::Seen,
    thread::Threads,
};

#[derive(Debug, Default)]
pub(crate) struct Mirror {
    url: String,
    dir: PathBuf,
    epochs: Vec<Epoch>,
    pub threads: Threads,
    seen: Seen,
    /// Never reset, so mails from before a rewrite stay ordered before the new ones
    next_seq: u64,
    /// Mails not handled yet, in archive order
    pub unseen: Vec<Mail>,
}

impl Mirror {
    pub fn open(url: String, dir: PathBuf) -> Result<Self> {
        Ok(Self {
            url,
            seen: Seen::read(&dir)?,
            dir,
            ..Self::default()
        })
    }

    /// Forget the index, the next one reports the mails after the seen one
    fn rewind(&mut self) {
        self.threads = Threads::default();
        self.unseen.clear();
        self.seen.rewind();
        for epoch in &mut self.epochs {
            epoch.indexed = None;
        }
    }

    /// Epochs from our newest one on, older ones are only fetched at startup
    pub fn to_fetch(&self, epochs: &[u32]) -> Result<Vec<Epoch>> {
        let newest = self.epochs.last().map_or(0, |epoch| epoch.number);

        epochs
            .iter()
            .filter(|&&number| number >= newest)
            .map(|&number| match self.epoch(number) {
                Some(epoch) => Ok(epoch.clone()),
                None => Epoch::open(&self.dir, number),
            })
            .collect()
    }

    /// Clones of known epochs were only needed for the fetch
    pub fn add(&mut self, epochs: Vec<Epoch>) {
        for epoch in epochs {
            if self.epoch(epoch.number).is_none() {
                self.epochs.push(epoch);
            }
        }
        self.epochs.sort_unstable_by_key(|epoch| epoch.number);
    }

    fn epoch(&self, number: u32) -> Option<&Epoch> {
        self.epochs.iter().find(|epoch| epoch.number == number)
    }

    pub fn index(&mut self) -> Result<()> {
        for position in 0..self.epochs.len() {
            if !self.index_epoch(position)? {
                // History was rewritten, index it again like after a restart
                self.rewind();
                return self.index();
            }
        }
        // The seen mail was purged, its successors can't be told from handled ones
        if self.seen.caught_up() {
            self.skip_pending()?;
        }
        Ok(())
    }

    /// `false` if history was rewritten
    fn index_epoch(&mut self, position: usize) -> Result<bool> {
        let Some(entries) = self.epochs[position].walk()? else {
            return Ok(false);
        };

        for entry in &entries {
            let Some(mut mail) = self.epochs[position].mail(entry, &self.url)? else {
                continue;
            };
            if entry.deleted {
                self.threads.remove(&mail.message_id);
                self.unseen
                    .retain(|unseen| unseen.message_id != mail.message_id);
                continue;
            }
            mail.seq = self.next_seq;
            self.next_seq += 1;

            let inserted = self.threads.insert(mail.clone());
            if self.seen.reached(&mail) {
                self.unseen.clear();
            } else if inserted {
                self.unseen.push(mail);
            }
        }

        if let Some(last) = entries.last() {
            self.epochs[position].indexed = Some(last.commit);
        }
        Ok(true)
    }

    /// The first start that completes marks the archive seen, even an empty one
    pub fn finish_first_start(&mut self) -> Result<()> {
        if self.seen.is_new() {
            self.skip_pending()?;
        }
        Ok(())
    }

    fn skip_pending(&mut self) -> Result<()> {
        self.seen.write(self.unseen.last())?;
        self.unseen.clear();
        Ok(())
    }

    pub fn mark_seen(&mut self, mail: &Mail) -> Result<()> {
        // Already behind the cursor: marked out of order, or from before a rewrite
        if self.unseen.iter().all(|unseen| unseen.seq > mail.seq) {
            return Ok(());
        }
        self.seen.write(Some(mail))?;
        // By order, `mail` itself may have been deleted since
        self.unseen.retain(|unseen| unseen.seq > mail.seq);
        Ok(())
    }

    pub fn message(&self, mail: &Mail) -> Result<Message<'static>> {
        let epoch = self
            .epoch(mail.epoch)
            .ok_or(MailingListError::EpochNotFound(mail.epoch))?;
        message::parse(epoch.blob(mail.blob)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    /// Like `MailingList::build` with epochs 0 and 1 already fetched
    fn start(dir: &Path) -> Mirror {
        let mut mirror = open(dir);
        mirror.index().unwrap();
        mirror.finish_first_start().unwrap();
        mirror
    }

    /// A start that dies before `finish_first_start`
    fn open(dir: &Path) -> Mirror {
        let mut mirror = Mirror::open("https://l/list/".to_string(), dir.to_path_buf()).unwrap();
        mirror.add(vec![
            Epoch::open(dir, 0).unwrap(),
            Epoch::open(dir, 1).unwrap(),
        ]);
        mirror
    }

    /// Like `MailingList::fetch` then `pending`
    fn sync(mirror: &mut Mirror) -> Vec<String> {
        mirror.index().unwrap();
        mirror
            .unseen
            .iter()
            .map(|mail| mail.message_id.clone())
            .collect()
    }

    /// The bot handled the pending mail `id`
    fn handled(mirror: &mut Mirror, id: &str) {
        let mail = mirror
            .unseen
            .iter()
            .find(|mail| mail.message_id == id)
            .cloned();
        mirror.mark_seen(&mail.unwrap()).unwrap();
    }

    #[test]
    fn only_epochs_from_the_newest_known_are_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let epochs = open(dir.path()).to_fetch(&[0, 1, 2]).unwrap();
        let numbers: Vec<u32> = epochs.iter().map(|epoch| epoch.number).collect();
        assert_eq!(numbers, [1, 2]);
    }

    #[test]
    fn first_start_marks_the_archive_seen() {
        let dir = tempfile::tempdir().unwrap();
        open(dir.path()).epochs[0].archive(&["a", "b"]);

        let mut mirror = start(dir.path());
        assert!(sync(&mut mirror).is_empty());
        assert_eq!(mirror.threads.all().len(), 2);

        mirror.epochs[0].archive(&["c"]);
        assert_eq!(sync(&mut mirror), ["c"]);
        handled(&mut mirror, "c");
        assert!(sync(&mut mirror).is_empty());
    }

    #[test]
    fn interrupted_first_start_does_not_report_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = open(dir.path());
        first.epochs[0].archive(&["a", "b"]);
        first.index().unwrap();

        assert!(sync(&mut start(dir.path())).is_empty());
    }

    #[test]
    fn mails_after_an_empty_first_start_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        start(dir.path()).epochs[0].archive(&["a"]);

        assert_eq!(sync(&mut start(dir.path())), ["a"]);
    }

    #[test]
    fn unhandled_mails_are_returned_again_and_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b", "c"]);
        assert_eq!(sync(&mut mirror), ["a", "b", "c"]);

        handled(&mut mirror, "a");
        assert_eq!(sync(&mut mirror), ["b", "c"]);
        assert_eq!(sync(&mut mirror), ["b", "c"]);
        assert_eq!(sync(&mut start(dir.path())), ["b", "c"]);
    }

    #[test]
    fn mail_is_read_back_from_the_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a"]);
        sync(&mut mirror);

        let message = mirror.message(&mirror.unseen[0]).unwrap();
        assert_eq!(message.body_text(0).as_deref(), Some("body of a"));
    }

    #[test]
    fn deleted_mail_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b"]);
        mirror.epochs[0].delete("a");

        assert_eq!(sync(&mut mirror), ["b"]);
        assert!(!mirror.threads.contains("a"));
    }

    #[test]
    fn rewritten_history_reports_the_mails_after_the_seen_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b"]);
        sync(&mut mirror);
        handled(&mut mirror, "a");

        // "b" is purged and "c" archived
        mirror.epochs[0].rewrite(&["a", "c"]);
        assert_eq!(sync(&mut mirror), ["c"]);
        assert!(!mirror.threads.contains("b"));
        assert_eq!(sync(&mut start(dir.path())), ["c"]);
    }

    #[test]
    fn purged_seen_mail_reports_nothing_until_new_mails() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b"]);
        sync(&mut mirror);
        handled(&mut mirror, "b");

        mirror.epochs[0].rewrite(&["a", "c"]);
        assert!(sync(&mut mirror).is_empty());

        mirror.epochs[0].archive(&["d"]);
        assert_eq!(sync(&mut mirror), ["d"]);
        assert_eq!(sync(&mut start(dir.path())), ["d"]);
    }

    #[test]
    fn rewrite_in_a_later_epoch_keeps_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a"]);
        mirror.epochs[1].archive(&["b", "c"]);
        assert_eq!(sync(&mut mirror), ["a", "b", "c"]);
        handled(&mut mirror, "a");

        // "b" is purged from epoch 1
        mirror.epochs[1].rewrite(&["c"]);
        assert_eq!(sync(&mut mirror), ["c"]);
        assert_eq!(sync(&mut start(dir.path())), ["c"]);
    }

    #[test]
    fn deleting_the_seen_mail_keeps_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a"]);
        sync(&mut mirror);
        handled(&mut mirror, "a");

        mirror.epochs[0].delete("a");
        mirror.epochs[0].archive(&["b"]);
        assert_eq!(sync(&mut mirror), ["b"]);
        assert_eq!(sync(&mut start(dir.path())), ["b"]);
    }

    #[test]
    fn readded_mails_are_told_apart_by_commit() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b"]);
        sync(&mut mirror);
        handled(&mut mirror, "a");

        // Only the first copy of "a" was handled
        mirror.epochs[0].delete("a");
        mirror.epochs[0].archive(&["a"]);
        assert_eq!(sync(&mut start(dir.path())), ["b", "a"]);

        sync(&mut mirror);
        handled(&mut mirror, "b");
        handled(&mut mirror, "a");
        assert!(sync(&mut start(dir.path())).is_empty());
    }

    #[test]
    fn first_start_with_a_readded_mail_marks_everything_seen() {
        let dir = tempfile::tempdir().unwrap();
        let first = open(dir.path());
        first.epochs[0].archive(&["a", "b"]);
        first.epochs[0].delete("a");
        first.epochs[0].archive(&["a"]);

        assert!(sync(&mut start(dir.path())).is_empty());
        assert!(sync(&mut start(dir.path())).is_empty());
    }

    #[test]
    fn mail_deleted_before_being_handled_still_drops_the_earlier_ones() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b", "c"]);
        sync(&mut mirror);
        let b = mirror.unseen[1].clone();

        mirror.epochs[0].delete("b");
        sync(&mut mirror);
        mirror.mark_seen(&b).unwrap();
        assert_eq!(sync(&mut mirror), ["c"]);
        assert_eq!(sync(&mut start(dir.path())), ["c"]);
    }

    #[test]
    fn marking_behind_the_cursor_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a", "b"]);
        mirror.epochs[1].archive(&["c", "d"]);
        sync(&mut mirror);
        let a = mirror.unseen[0].clone();

        handled(&mut mirror, "c");
        mirror.mark_seen(&a).unwrap();
        assert_eq!(sync(&mut mirror), ["d"]);
        assert_eq!(sync(&mut start(dir.path())), ["d"]);
    }

    #[test]
    fn marking_a_mail_from_before_a_rewrite_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["x", "a"]);
        mirror.epochs[1].archive(&["y"]);
        sync(&mut mirror);
        handled(&mut mirror, "x");
        let a = mirror.unseen[0].clone();

        mirror.epochs[1].rewrite(&["z"]);
        assert_eq!(sync(&mut mirror), ["a", "z"]);
        mirror.mark_seen(&a).unwrap();
        assert_eq!(sync(&mut mirror), ["a", "z"]);
        assert_eq!(sync(&mut start(dir.path())), ["a", "z"]);
    }

    #[test]
    fn seen_file_without_a_commit_falls_back_to_the_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = start(dir.path());
        mirror.epochs[0].archive(&["a b", "c"]);
        sync(&mut mirror);
        handled(&mut mirror, "a b");
        assert_eq!(sync(&mut start(dir.path())), ["c"]);

        for seen in ["a b", "a b\n"] {
            fs::write(dir.path().join("seen"), seen).unwrap();
            assert_eq!(sync(&mut start(dir.path())), ["c"], "{seen:?}");
        }
    }
}
