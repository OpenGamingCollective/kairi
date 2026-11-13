//! The mirror indexes the git on disk, the reader serves lists and threads from that index

mod common;

use std::{fs, path::Path};

use common::Epoch;
use public_inbox::{List, MailingListError, Mirror, Reader};

const DESCRIPTION: &str = "A list";

/// Index the git in `dir` like the mirror does after a fetch
fn reindex(dir: &Path) -> bool {
    Mirror::open("https://l/list/", dir, "test")
        .unwrap()
        .reindex(DESCRIPTION)
        .unwrap()
}

fn list(dir: &Path) -> List {
    Reader::open(dir).fetch_list().unwrap()
}

fn names(list: &List) -> Vec<&str> {
    list.threads
        .iter()
        .map(|thread| thread.name.as_str())
        .collect()
}

fn index_target(dir: &Path) -> std::path::PathBuf {
    fs::read_link(dir.join("index")).unwrap()
}

#[test]
fn the_list_is_read_from_the_index() {
    let dir = tempfile::tempdir().unwrap();
    Epoch::open(dir.path(), 0).archive(&["a", "b"]);
    Epoch::open(dir.path(), 1).archive(&["c"]);
    assert!(reindex(dir.path()));

    let list = list(dir.path());
    let mut names = names(&list);
    names.sort_unstable();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(list.description, DESCRIPTION);
    assert_eq!(list.threads[0].url, "https://l/list/a/T/");
    assert_eq!(list.threads[0].mails, 1);
}

#[test]
fn nothing_is_read_before_the_first_index() {
    let dir = tempfile::tempdir().unwrap();
    Epoch::open(dir.path(), 0).archive(&["a"]);

    let list = list(dir.path());
    assert!(list.threads.is_empty());
    assert_eq!(list.description, "");
    assert!(
        Reader::open(dir.path())
            .fetch_thread("abc")
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_empty_mirror_is_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    assert!(reindex(dir.path()));

    let list = list(dir.path());
    assert!(list.threads.is_empty());
    assert_eq!(list.description, DESCRIPTION);
}

#[test]
fn fetch_thread_returns_the_mails_of_one_thread() {
    let dir = tempfile::tempdir().unwrap();
    Epoch::open(dir.path(), 0).archive(&["a", "b"]);
    reindex(dir.path());

    let summary = list(dir.path()).threads[0].clone();
    let thread = Reader::open(dir.path())
        .fetch_thread(&summary.uid)
        .unwrap()
        .unwrap();

    assert_eq!(thread.uid(), summary.uid);
    assert_eq!(thread.name(), summary.name);
    assert_eq!(thread.mails()[0].message_id, "a");
}

#[test]
fn uids_never_leave_the_index() {
    let dir = tempfile::tempdir().unwrap();
    Epoch::open(dir.path(), 0).archive(&["a"]);
    reindex(dir.path());
    fs::write(dir.path().join("secret.json"), "{}").unwrap();

    let reader = Reader::open(dir.path());
    for uid in ["../../secret", "", "zz", "0123456789ab"] {
        assert!(reader.fetch_thread(uid).unwrap().is_none(), "{uid:?}");
    }
}

#[test]
fn an_unchanged_git_is_not_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let epoch = Epoch::open(dir.path(), 0);
    epoch.archive(&["a"]);
    assert!(reindex(dir.path()));
    let target = index_target(dir.path());

    assert!(!reindex(dir.path()));
    assert_eq!(index_target(dir.path()), target);

    epoch.archive(&["b"]);
    assert!(reindex(dir.path()));
    assert_ne!(index_target(dir.path()), target);
    // The previous index is removed once swapped out
    assert!(!dir.path().join(target).exists());
    assert_eq!(names(&list(dir.path())).len(), 2);
}

#[test]
fn a_broken_index_is_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    Epoch::open(dir.path(), 0).archive(&["a", "b"]);
    reindex(dir.path());
    let uid = list(dir.path()).threads[1].uid.clone();

    // A power loss left a thread file empty, the heads still match
    let thread = dir.path().join("index/threads").join(format!("{uid}.json"));
    fs::write(&thread, "").unwrap();
    assert!(Reader::open(dir.path()).fetch_thread(&uid).is_err());
    assert!(reindex(dir.path()));
    assert!(
        Reader::open(dir.path())
            .fetch_thread(&uid)
            .unwrap()
            .is_some()
    );

    // Or `list.json` itself
    fs::write(dir.path().join("index/list.json"), "").unwrap();
    assert!(reindex(dir.path()));
    assert_eq!(names(&list(dir.path())), ["a", "b"]);

    // Or a thread file is missing
    fs::remove_file(dir.path().join("index/threads").join(format!("{uid}.json"))).unwrap();
    assert!(reindex(dir.path()));
    assert!(!reindex(dir.path()));
}

#[test]
fn a_new_description_is_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open("https://l/list/", dir.path(), "test").unwrap();
    assert!(mirror.reindex("old").unwrap());
    assert!(mirror.reindex("new").unwrap());
    assert_eq!(list(dir.path()).description, "new");
}

#[test]
fn deleted_mail_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let epoch = Epoch::open(dir.path(), 0);
    epoch.archive(&["a", "b"]);
    epoch.delete("a");
    reindex(dir.path());

    assert_eq!(names(&list(dir.path())), ["b"]);
}

#[test]
fn rewritten_history_drops_purged_threads() {
    let dir = tempfile::tempdir().unwrap();
    let epoch = Epoch::open(dir.path(), 0);
    epoch.archive(&["a", "b"]);
    reindex(dir.path());
    let purged = list(dir.path()).threads[1].uid.clone();

    epoch.rewrite(&["a", "c"]);
    assert!(reindex(dir.path()));

    assert_eq!(names(&list(dir.path())), ["a", "c"]);
    assert!(
        Reader::open(dir.path())
            .fetch_thread(&purged)
            .unwrap()
            .is_none()
    );
}

#[test]
fn mail_is_read_back_from_the_git() {
    let dir = tempfile::tempdir().unwrap();
    Epoch::open(dir.path(), 0).archive(&["a"]);
    reindex(dir.path());

    let reader = Reader::open(dir.path());
    let uid = list(dir.path()).threads[0].uid.clone();
    let thread = reader.fetch_thread(&uid).unwrap().unwrap();
    let message = reader.message(&thread.mails()[0]).unwrap();
    assert_eq!(message.body_text(0).as_deref(), Some("body of a"));

    // The mail's epoch is gone from the mirror
    fs::remove_dir_all(dir.path().join("git/0.git")).unwrap();
    assert!(matches!(
        reader.message(&thread.mails()[0]),
        Err(MailingListError::EpochNotFound(0))
    ));
}
