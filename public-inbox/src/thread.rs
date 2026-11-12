//! Threading of the mails of a mailing list
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::message::Mail;

/// Representation of a mailing list thread
#[derive(Debug, Clone)]
pub struct Thread {
    uid: String,
    mails: Vec<Mail>,
}

impl Thread {
    fn new(first_message_id: &str) -> Self {
        let uid = Sha256::digest(first_message_id)[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            uid,
            mails: Vec::new(),
        }
    }

    /// Hash of the first archived mail's Message-ID, stable unless that mail is
    /// purged or the thread is merged into another
    pub fn uid(&self) -> &str {
        &self.uid
    }

    pub fn name(&self) -> &str {
        &self.root().subject
    }

    pub fn author(&self) -> &str {
        &self.root().author
    }

    pub fn date(&self) -> DateTime<Utc> {
        self.root().date
    }

    pub fn url(&self) -> String {
        format!("{}T/", self.root().url)
    }

    /// Initiating email of the thread
    pub fn root(&self) -> &Mail {
        &self.mails[0]
    }

    /// Every mail of the thread, root first
    pub fn mails(&self) -> &[Mail] {
        &self.mails
    }
}

/// Every thread of a mailing list, emptied threads are kept but hidden
#[derive(Debug, Default)]
pub(crate) struct Threads {
    threads: Vec<Thread>,
    /// Also maps the ids replied to, even if they are not archived (yet)
    by_message_id: HashMap<String, usize>,
}

impl Threads {
    /// Returns `false` when the mail is already known
    pub fn insert(&mut self, mail: Mail) -> bool {
        if self.contains(&mail.message_id) {
            return false;
        }

        // Threads of its parents first, then the one its replies started
        let linked: Vec<usize> = mail
            .parents
            .iter()
            .filter(|&id| *id != mail.message_id)
            .chain([&mail.message_id])
            .filter_map(|id| self.by_message_id.get(id).copied())
            .collect();
        let position = linked.first().copied().unwrap_or_else(|| {
            self.threads.push(Thread::new(&mail.message_id));
            self.threads.len() - 1
        });
        // This mail links them all into one
        for &other in linked.iter().skip(1) {
            if other != position {
                self.merge(other, position);
            }
        }

        self.by_message_id.insert(mail.message_id.clone(), position);
        for parent in &mail.parents {
            self.by_message_id.entry(parent.clone()).or_insert(position);
        }

        // A parent archived after its replies becomes the root
        let mails = &mut self.threads[position].mails;
        if mails
            .first()
            .is_none_or(|root| root.parents.contains(&mail.message_id))
        {
            mails.insert(0, mail);
        } else {
            mails.push(mail);
        }
        true
    }

    /// Move the mails of thread `from` into thread `into`
    fn merge(&mut self, from: usize, into: usize) {
        let mails = std::mem::take(&mut self.threads[from].mails);
        self.threads[into].mails.extend(mails);
        for position in self.by_message_id.values_mut() {
            if *position == from {
                *position = into;
            }
        }
    }

    pub fn remove(&mut self, message_id: &str) {
        if let Some(&position) = self.by_message_id.get(message_id) {
            self.threads[position]
                .mails
                .retain(|mail| mail.message_id != message_id);
        }
    }

    /// Whether a mail is archived
    pub fn contains(&self, message_id: &str) -> bool {
        self.by_message_id.get(message_id).is_some_and(|&position| {
            self.threads[position]
                .mails
                .iter()
                .any(|mail| mail.message_id == message_id)
        })
    }

    pub fn get(&self, uid: &str) -> Option<&Thread> {
        self.visible().find(|thread| thread.uid == uid)
    }

    /// Threads sorted from the oldest to the newest
    pub fn all(&self) -> Vec<Thread> {
        let mut threads: Vec<Thread> = self.visible().cloned().collect();
        threads.sort_by_key(Thread::date);
        threads
    }

    fn visible(&self) -> impl Iterator<Item = &Thread> {
        self.threads
            .iter()
            .filter(|thread| !thread.mails.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use gix::ObjectId;

    use super::*;

    fn mail(id: &str, parents: &[&str]) -> Mail {
        Mail {
            message_id: id.to_string(),
            subject: format!("subject of {id}"),
            author: "alice".to_string(),
            sender: None,
            date: DateTime::default(),
            url: format!("https://l/list/{id}/"),
            epoch: 0,
            blob: ObjectId::null(gix::hash::Kind::Sha1),
            commit: ObjectId::null(gix::hash::Kind::Sha1),
            seq: 0,
            parents: parents.iter().map(ToString::to_string).collect(),
        }
    }

    fn threads(mails: &[(&str, &[&str])]) -> Threads {
        let mut threads = Threads::default();
        for (id, parents) in mails {
            threads.insert(mail(id, parents));
        }
        threads
    }

    fn ids(thread: &Thread) -> Vec<&str> {
        thread
            .mails()
            .iter()
            .map(|mail| &*mail.message_id)
            .collect()
    }

    #[test]
    fn replies_join_the_thread_of_their_parent() {
        let threads = threads(&[("a", &[]), ("b", &["a"]), ("c", &["b", "a"]), ("d", &[])]);

        let all = threads.all();
        assert_eq!(all.len(), 2);
        let thread = all
            .iter()
            .find(|thread| thread.name() == "subject of a")
            .unwrap();
        assert_eq!(ids(thread), ["a", "b", "c"]);
        assert_eq!(thread.url(), "https://l/list/a/T/");
        assert_eq!(
            threads.get(thread.uid()).map(ids),
            Some(vec!["a", "b", "c"])
        );
    }

    #[test]
    fn known_mail_is_not_inserted_twice() {
        let mut threads = threads(&[("a", &[])]);
        assert!(!threads.insert(mail("a", &[])));
        assert_eq!(threads.all()[0].mails().len(), 1);
    }

    #[test]
    fn root_archived_after_its_replies_takes_over_the_thread() {
        let mut threads = threads(&[("b", &["a"]), ("c", &["a"])]);
        let uid = threads.all()[0].uid().to_string();

        threads.insert(mail("a", &[]));
        assert_eq!(ids(&threads.all()[0]), ["a", "b", "c"]);
        assert_eq!(threads.all()[0].uid(), uid);
    }

    #[test]
    fn reversed_chain_is_rebuilt() {
        let threads = threads(&[("c", &["b", "a"]), ("b", &["a"]), ("a", &[])]);
        assert_eq!(ids(&threads.all()[0]), ["a", "b", "c"]);
    }

    #[test]
    fn only_an_ancestor_of_the_root_takes_over() {
        let late_reply = threads(&[("a", &[]), ("c", &["b", "a"]), ("b", &["a"])]);
        assert_eq!(ids(&late_reply.all()[0]), ["a", "c", "b"]);

        let stripped = threads(&[("a", &[]), ("c", &["b", "a"]), ("b", &[])]);
        assert_eq!(ids(&stripped.all()[0]), ["a", "c", "b"]);

        let cousin = threads(&[("b", &["a"]), ("c", &["d", "a"]), ("d", &[])]);
        assert_eq!(ids(&cousin.all()[0]), ["b", "c", "d"]);
    }

    #[test]
    fn late_middle_mail_joins_both_halves() {
        let all = threads(&[("a", &[]), ("c", &["b"]), ("b", &["a"])]).all();
        assert_eq!(all.len(), 1);
        assert_eq!(ids(&all[0]), ["a", "c", "b"]);

        let siblings = threads(&[("x", &["p"]), ("b", &["a"]), ("a", &["p"])]);
        assert_eq!(ids(&siblings.all()[0]), ["x", "b", "a"]);
    }

    #[test]
    fn mail_linking_several_threads_merges_them() {
        let self_reference = threads(&[("a", &[]), ("c", &["b"]), ("b", &["b", "a"])]);
        assert_eq!(ids(&self_reference.all()[0]), ["a", "c", "b"]);

        let two_parents = threads(&[("a", &[]), ("x", &[]), ("m", &["a", "x"])]);
        assert_eq!(ids(&two_parents.all()[0]), ["a", "x", "m"]);
    }

    #[test]
    fn same_subject_and_date_are_distinct_threads() {
        let mut threads = threads(&[("a", &[])]);
        let mut twin = mail("b", &[]);
        twin.subject = "subject of a".to_string();
        threads.insert(twin);
        assert_eq!(threads.all().len(), 2);
    }

    #[test]
    fn removed_mail_leaves_its_thread() {
        let mut threads = threads(&[("a", &[]), ("b", &["a"])]);

        threads.remove("a");
        assert!(!threads.contains("a"));
        assert_eq!(threads.all()[0].name(), "subject of b");

        threads.remove("b");
        assert!(threads.all().is_empty());

        threads.insert(mail("a", &[]));
        assert_eq!(threads.all()[0].name(), "subject of a");
    }
}
