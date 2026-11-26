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

pub const UNAFFILIATED: &str = "unaffiliated";

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
    /// Only organisations vote. Other groups are projects
    #[serde(default)]
    org: bool,
    #[serde(default)]
    members: Vec<String>,
}

impl Member {
    /// Only members with a valid email can vote
    fn can_vote(&self) -> bool {
        let email = self.email.trim();
        let whole = !email.contains(char::is_whitespace);
        let parts = email.split_once('@');
        whole && parts.is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
    }
}

/// An organisation, or the block of unaffiliated members
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub slug: String,
    pub name: String,
    pub unaffiliated: bool,
    /// Usernames of its members who can vote
    pub members: BTreeSet<String>,
}

impl Roster {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Number of members, with or without an email
    pub fn members(&self) -> usize {
        self.members.len()
    }

    /// Number of members with a valid email
    pub fn eligible(&self) -> usize {
        self.members
            .values()
            .filter(|member| member.can_vote())
            .count()
    }

    fn can_vote(&self, username: &str) -> bool {
        self.members.get(username).is_some_and(Member::can_vote)
    }

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

    /// Organisations sorted by slug, followed by the unaffiliated block
    ///
    /// Only members who can vote are included, each in the first of their organisations by
    /// slug. Entities left with no members are skipped
    pub fn entities(&self) -> Vec<Entity> {
        let mut affiliated: BTreeSet<&String> = BTreeSet::new();
        let mut entities: Vec<Entity> = Vec::new();
        for (slug, group) in self.groups.iter().filter(|(_, group)| group.org) {
            let mut members = BTreeSet::new();
            for username in &group.members {
                if self.can_vote(username) && affiliated.insert(username) {
                    members.insert(username.clone());
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
                slug: slug.clone(),
                name: name.clone(),
                unaffiliated: false,
                members,
            });
        }

        let unaffiliated: BTreeSet<String> = self
            .members
            .keys()
            .filter(|username| self.can_vote(username))
            .filter(|username| !affiliated.contains(username))
            .cloned()
            .collect();
        if !unaffiliated.is_empty() {
            entities.push(Entity {
                slug: UNAFFILIATED.to_string(),
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
        let slugs: Vec<&str> = entities.iter().map(|entity| &*entity.slug).collect();
        assert_eq!(slugs, ["fyra", "ublue", UNAFFILIATED]);
        assert_eq!(entities[0].name, "Fyra Labs");
        assert_eq!(entities[1].name, "ublue");
    }

    #[test]
    fn members_outside_of_an_organisation_are_unaffiliated() {
        let entities = Roster::parse(ROSTER).unwrap().entities();
        let block = entities.last().unwrap();
        assert!(block.unaffiliated);
        // dave is only in a project
        assert_eq!(block.members.iter().collect::<Vec<_>>(), ["carol", "dave"]);
    }

    #[test]
    fn a_member_only_belongs_to_their_first_organisation() {
        let entities = Roster::parse(ROSTER).unwrap().entities();
        assert!(entities[0].members.contains("alice"));
        assert_eq!(entities[1].members.iter().collect::<Vec<_>>(), ["erin"]);
        // bob was its only member, and bob can't vote
        assert!(entities.iter().all(|entity| entity.slug != "zeta"));
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
        assert_eq!(roster.members(), 5);
        assert_eq!(roster.eligible(), 4);
    }

    #[test]
    fn only_members_with_a_valid_email_are_in_an_entity() {
        let roster = Roster::parse(
            r#"{
                "members": {
                    "alice": { "email": "alice@example.org" },
                    "bob": { "email": "" },
                    "carol": { "email": "not an email" },
                    "dave": { "email": "dave@" },
                    "erin": { "email": " erin@example.org " },
                    "frank": {}
                },
                "groups": {
                    "fyra": { "org": true, "members": ["alice", "bob", "carol", "stranger"] },
                    "ublue": { "org": true, "members": ["dave"] }
                }
            }"#,
        )
        .unwrap();
        assert_eq!((roster.members(), roster.eligible()), (6, 2));

        let entities = roster.entities();
        let members = |entity: &Entity| entity.members.iter().cloned().collect::<Vec<_>>();
        // Nobody in ublue can vote, and frank isn't in the unaffiliated block
        assert_eq!(entities.len(), 2);
        assert_eq!(
            (&*entities[0].slug, members(&entities[0])),
            ("fyra", vec!["alice".to_string()])
        );
        assert_eq!(
            (&*entities[1].slug, members(&entities[1])),
            (UNAFFILIATED, vec!["erin".to_string()])
        );
    }

    #[test]
    fn invalid_json_errors() {
        assert!(Roster::parse("not json").is_err());
        assert!(Roster::parse("{}").is_err());
    }
}
