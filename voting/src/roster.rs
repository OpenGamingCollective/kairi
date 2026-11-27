//! Members and organisations of the OGC, from the infra inventory
//!
//! The roster is both inventory files of the infra repo, as JSON:
//!
//! ```text
//! nix eval --impure --json --expr '{
//!   members = (import ./modules/inventory/members.nix { })._module.args.members;
//!   groups = (import ./modules/inventory/groups.nix { })._module.args.groups;
//! }'
//! ```
//!
//! ```json
//! {
//!   "members": { "<username>": { "email": "", "github": "" } },
//!   "groups": { "<slug>": { "description": "", "org": true, "members": ["<username>"] } }
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Roster {
    members: BTreeMap<String, Member>,
    #[serde(default)]
    groups: BTreeMap<String, Group>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Member {
    #[serde(default)]
    email: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Group {
    #[serde(default)]
    description: String,
    /// Only organisations vote
    #[serde(default)]
    org: bool,
    #[serde(default)]
    members: Vec<String>,
}

impl Member {
    /// Only members with an email can vote
    fn can_vote(&self) -> bool {
        !self.email.trim().is_empty()
    }
}

/// An organisation or the unaffiliated block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub unaffiliated: bool,
    pub members: Vec<String>,
}

impl Roster {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Total number of members
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Number of members with an email
    pub fn eligible_count(&self) -> usize {
        self.members
            .values()
            .filter(|member| member.can_vote())
            .count()
    }

    fn can_vote(&self, username: &str) -> bool {
        self.members.get(username).is_some_and(Member::can_vote)
    }

    /// Find member by email
    pub fn member_of(&self, email: &str) -> Option<&str> {
        let email = email.trim();
        if email.is_empty() {
            return None;
        }
        self.members
            .iter()
            .find(|(_, member)| member.email.trim().eq_ignore_ascii_case(email))
            .map(|(username, _)| username.as_str())
    }

    /// Organisations + unaffiliated block
    pub fn entities(&self) -> Vec<Entity> {
        let mut seen: BTreeSet<&String> = BTreeSet::new();
        let mut entities: Vec<Entity> = Vec::new();

        // Groups here are sorted alphabetically so Nero stays in ASUS Linux and not in ChimeraOS
        for (slug, group) in self.groups.iter().filter(|(_, group)| group.org) {
            let mut members = Vec::new();
            for username in &group.members {
                // This will fail if the member has already been seen
                if self.can_vote(username) && seen.insert(username) {
                    members.push(username.clone());
                }
            }
            if members.is_empty() {
                continue;
            }
            let name = if group.description.is_empty() {
                slug
            } else {
                &group.description
            };
            entities.push(Entity {
                name: name.clone(),
                unaffiliated: false,
                members,
            });
        }

        let unaffiliated: Vec<String> = self
            .members
            .keys()
            .filter(|username| self.can_vote(username))
            .filter(|username| !seen.contains(username))
            .cloned()
            .collect();
        if !unaffiliated.is_empty() {
            entities.push(Entity {
                name: "Unaffiliated".to_string(),
                unaffiliated: true,
                members: unaffiliated,
            });
        }
        entities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROSTER: &str = r#"{
        "members": {
            "alice": { "email": "Alice@Example.org", "github": "alice", "orgRole": "admin" },
            "bob": { "email": "", "github": "" },
            "carol": { "email": "carol@example.org" },
            "dave": { "email": "dave@example.org" },
            "erin": { "email": "erin@example.org" }
        },
        "groups": {
            "kernel": { "description": "Kernel", "org": false, "members": ["dave"] },
            "fyra": { "description": "Fyra Labs", "org": true, "members": ["alice", "bob"] },
            "ublue": { "description": "", "org": true, "members": ["alice", "erin"] },
            "zeta": { "description": "Zeta", "org": true, "members": ["bob"] },
            "empty": { "description": "Empty", "org": true, "members": [] }
        }
    }"#;

    #[test]
    fn organisations_are_the_groups_flagged_org() {
        let entities = Roster::parse(ROSTER).unwrap().entities();
        let names: Vec<&str> = entities.iter().map(|entity| &*entity.name).collect();
        assert_eq!(names, ["Fyra Labs", "ublue", "Unaffiliated"]);
    }

    #[test]
    fn members_outside_of_an_organisation_are_unaffiliated() {
        let entities = Roster::parse(ROSTER).unwrap().entities();
        let block = entities.last().unwrap();
        assert!(block.unaffiliated);
        // dave is only in a project
        assert_eq!(block.members, ["carol", "dave"]);
    }

    #[test]
    fn a_member_only_belongs_to_their_first_organisation() {
        let entities = Roster::parse(ROSTER).unwrap().entities();
        assert!(entities[0].members.contains(&"alice".to_string()));
        assert_eq!(entities[1].members, ["erin"]);
        // bob was its only member, and bob can't vote
        assert!(entities.iter().all(|entity| entity.name != "Zeta"));
    }

    #[test]
    fn members_are_found_by_their_email_whatever_its_case() {
        let roster = Roster::parse(ROSTER).unwrap();
        assert_eq!(roster.member_of("alice@example.org"), Some("alice"));
        assert_eq!(roster.member_of(" CAROL@example.org "), Some("carol"));
        assert_eq!(roster.member_of("mallory@example.org"), None);
    }

    #[test]
    fn a_missing_email_matches_nobody() {
        let roster = Roster::parse(ROSTER).unwrap();
        assert_eq!(roster.member_of(""), None);
        assert_eq!(roster.member_count(), 5);
        assert_eq!(roster.eligible_count(), 4);
    }

    #[test]
    fn invalid_json_errors() {
        assert!(Roster::parse("not json").is_err());
        assert!(Roster::parse("{}").is_err());
    }
}
