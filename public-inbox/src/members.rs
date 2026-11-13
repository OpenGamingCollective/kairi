use std::collections::HashMap;

use serde::Deserialize;

use crate::errors::Result;

#[derive(Debug, Clone, Default)]
pub struct OrgList(pub HashMap<String, Vec<Member>>);

#[derive(Debug, Clone, Default)]
pub struct Member {
    pub name: String,
    pub email: String,
    pub github: String,
}

#[derive(Deserialize)]
struct JsonMembers(HashMap<String, JsonEntry>);

#[derive(Deserialize)]
struct JsonEntry {
    #[serde(default)]
    email: String,
    #[serde(default)]
    github: String,
    #[serde(default)]
    groups: Vec<String>,
}

pub fn parse_members_json(content: &str) -> Result<OrgList> {
    let json: JsonMembers = serde_json::from_str(content)?;

    let mut orgs = OrgList::default();

    for (name, entry) in json.0 {
        let org = entry
            .groups
            .first()
            .map_or("unaffiliated", |org| org.as_str());

        orgs.0.entry(org.to_string()).or_default().push(Member {
            name,
            email: entry.email,
            github: entry.github,
        });
    }

    Ok(orgs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org_names(orgs: &OrgList) -> Vec<&str> {
        let mut names: Vec<&str> = orgs.0.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn empty_email_member_stays_in_org() {
        let orgs = parse_members_json(
            r#"{"alice":{"email":"alice@example.org","groups":["fyra"]},"bob":{"email":"","groups":["fyra"]}}"#,
        )
        .unwrap();

        let members = &orgs.0["fyra"];
        assert_eq!(members.len(), 2);
        assert!(
            members
                .iter()
                .any(|member| member.name == "bob" && member.email.is_empty())
        );
    }

    #[test]
    fn missing_email_member_stays_in_org() {
        let orgs = parse_members_json(r#"{"bob":{"groups":["fyra"]}}"#).unwrap();

        assert_eq!(orgs.0["fyra"].len(), 1);
        assert_eq!(orgs.0["fyra"][0].email, "");
    }

    #[test]
    fn org_is_the_first_group() {
        let orgs = parse_members_json(
            r#"{"alice":{"email":"alice@example.org","groups":["fyra","kernel"]}}"#,
        )
        .unwrap();

        assert_eq!(org_names(&orgs), ["fyra"]);
        assert_eq!(orgs.0["fyra"][0].name, "alice");
    }

    #[test]
    fn empty_groups_fall_back_to_unaffiliated() {
        let orgs = parse_members_json(r#"{"alice":{"email":"","groups":[]}}"#).unwrap();

        assert_eq!(org_names(&orgs), ["unaffiliated"]);
        assert_eq!(orgs.0["unaffiliated"].len(), 1);
    }

    #[test]
    fn invalid_json_errors() {
        assert!(parse_members_json("not json").is_err());
    }
}
