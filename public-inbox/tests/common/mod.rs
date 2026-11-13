//! Write epochs the way public-inbox does: one bare repository per epoch, each commit
//! holding a single mail as `m`, or as `d` when it deletes it

use std::path::Path;

use gix::{
    ObjectId, Repository,
    objs::{Commit, Tree, tree},
    refs::transaction::PreviousValue,
};

const MASTER: &str = "refs/heads/master";

pub struct Epoch {
    repo: Repository,
}

impl Epoch {
    /// Create or open `<dir>/git/<number>.git`
    pub fn open(dir: &Path, number: u32) -> Self {
        let path = dir.join(format!("git/{number}.git"));
        if !path.exists() {
            std::fs::create_dir_all(&path).unwrap();
            gix::init_bare(&path).unwrap();
        }
        let repo = gix::open_opts(&path, gix::open::Options::isolated()).unwrap();
        Self { repo }
    }

    fn head(&self) -> Option<ObjectId> {
        self.repo
            .try_find_reference(MASTER)
            .unwrap()
            .and_then(|head| head.try_id().map(gix::Id::detach))
    }

    /// Commit a mail as the only file of the tree, on top of `parent`
    fn commit(&self, parent: Option<ObjectId>, file: &str, id: &str) -> ObjectId {
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
            self.commit(self.head(), "m", id);
        }
    }

    pub fn delete(&self, id: &str) {
        self.commit(self.head(), "d", id);
    }

    /// Replace the history, like a purge does
    pub fn rewrite(&self, ids: &[&str]) {
        let mut parent = None;
        for id in ids {
            parent = Some(self.commit(parent, "m", id));
        }
    }
}
