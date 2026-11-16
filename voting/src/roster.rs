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

/// Slug of the unaffiliated block
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

/// An organisation, or the block of unaffiliated members
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub slug: String,
    pub name: String,
    pub unaffiliated: bool,
    /// Usernames, a member can be on the roster of several organisations
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

    /// Members who gave an email, the others can't vote
    pub fn eligible(&self) -> usize {
        self.members
            .values()
            .filter(|member| !member.email.trim().is_empty())
            .count()
    }

    /// Username of the member with this address
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

    /// Organisations sorted by slug then the unaffiliated block, the empty ones are left out
    pub fn entities(&self) -> Vec<Entity> {
        let mut entities: Vec<Entity> = self
            .groups
            .iter()
            .filter(|(_, group)| group.org && !group.members.is_empty())
            .map(|(slug, group)| Entity {
                slug: slug.clone(),
                name: if group.description.is_empty() {
                    slug.clone()
                } else {
                    group.description.clone()
                },
                unaffiliated: false,
                members: group.members.iter().cloned().collect(),
            })
            .collect();

        let unaffiliated: BTreeSet<String> = self
            .members
            .keys()
            .filter(|username| {
                !entities
                    .iter()
                    .any(|entity| entity.members.contains(*username))
            })
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
            "dave": { "email": "dave@example.org" }
        },
        "groups": {
            "kernel": { "description": "Kernel", "org": false, "members": ["dave"] },
            "fyra": { "description": "Fyra Labs", "org": true, "members": ["alice", "bob"] },
            "ublue": { "description": "", "org": true, "members": ["alice"] },
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
    fn a_member_is_on_the_roster_of_each_of_their_organisations() {
        let entities = Roster::parse(ROSTER).unwrap().entities();
        assert!(entities[0].members.contains("alice"));
        assert!(entities[1].members.contains("alice"));
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
        assert_eq!(roster.members(), 4);
        assert_eq!(roster.eligible(), 3);
    }

    #[test]
    fn invalid_json_errors() {
        assert!(Roster::parse("not json").is_err());
        assert!(Roster::parse("{}").is_err());
    }
}
