//! Team tags: groups of people in a team that can be @mentioned (`@tag`).
//!
//! The endpoints are on the chat service aggregator (`CSA/api/v1/teams/<team thread id>/tags`),
//! found in the web client's `services-people-targeting` chunk and checked live. A mention of a
//! tag uses the mri `tag:<tagId>`.

use crate::client::Client;
use crate::error::{Error, Result};
use crate::mentions::{Mention, MentionKind};
use crate::teams::CSA_V1;

/// CSA v2 base, used to edit a tag.
pub const CSA_V2: &str = "https://teams.microsoft.com/api/csa/api/v2";
use reqwest::Method;
use serde_json::{Value, json};

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// A tag of a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub description: String,
    pub member_count: u32,
    pub member_ids: Vec<String>,
    /// We are one of the tag's members.
    pub is_member: bool,
    /// The version the next change must quote in `If-Match`.
    pub version: u64,
}

impl Tag {
    /// The mri a message mention of this tag carries.
    pub fn mri(&self) -> String {
        tag_mri(&self.id)
    }

    /// The mention for this tag. `item_id` is the position of its span in the message HTML.
    pub fn mention(&self, item_id: u32) -> Mention {
        Mention {
            item_id,
            mri: self.mri(),
            display_name: self.name.clone(),
            kind: MentionKind::Tag,
        }
    }
}

/// `tag:<id>`, the mri of a tag mention.
pub fn tag_mri(tag_id: &str) -> String {
    format!("tag:{tag_id}")
}

/// The tag id of a `tag:<id>` mri.
pub fn tag_id_of_mri(mri: &str) -> Option<&str> {
    mri.strip_prefix("tag:").filter(|s| !s.is_empty())
}

fn parse_tag(v: &Value) -> Option<Tag> {
    let id = v.get("tagId")?.as_str()?.to_string();
    Some(Tag {
        name: v
            .get("tagName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        description: v
            .get("tagDescription")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        member_count: v.get("memberCount").and_then(Value::as_u64).unwrap_or(0) as u32,
        member_ids: v
            .get("memberIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str().map(str::to_owned))
            .collect(),
        is_member: v
            .get("currentMemberIsPartOfTag")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        version: v.get("version").and_then(Value::as_u64).unwrap_or(0),
        id,
    })
}

/// Parses `GET …/teams/<t>/tags` (a bare array) or a single tag object.
pub fn parse_tags(v: &Value) -> Vec<Tag> {
    match v {
        Value::Array(a) => a.iter().filter_map(parse_tag).collect(),
        other => parse_tag(other).into_iter().collect(),
    }
}

/// A team with its tags, from the cross-team list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamTags {
    pub team_id: String,
    pub tags: Vec<Tag>,
}

/// Parses `GET …/teams/users/me/teams/tagCards`.
pub fn parse_tag_cards(v: &Value) -> Vec<TeamTags> {
    v.get("teamTagCards")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(TeamTags {
                team_id: t.get("teamId")?.as_str()?.to_string(),
                tags: t
                    .get("tagCards")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(parse_tag)
                    .collect(),
            })
        })
        .collect()
}

/// Parses `GET …/tags/<id>/members`: `members[].memberId`.
pub fn parse_tag_members(v: &Value) -> Vec<String> {
    v.get("members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("memberId").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// Body of `POST …/teams/<t>/tags`. `members` are mris.
pub fn create_tag_body(name: &str, description: &str, members: &[String]) -> Value {
    json!({
        "tagName": name,
        "tagDescription": description,
        "memberIds": members,
        "showInSection": false,
    })
}

/// Body of `PATCH …/teams/<t>/tags/<id>`: only the fields that change.
pub fn edit_tag_body(name: Option<&str>, description: Option<&str>) -> Value {
    let mut b = json!({});
    if let Some(n) = name {
        b["tagName"] = Value::from(n);
    }
    if let Some(d) = description {
        b["tagDescription"] = Value::from(d);
    }
    b
}

/// Body of `POST …/memberTags?action=add`: put people into tags, by tag name.
pub fn add_to_tags_body(members: &[String], tag_names: &[String]) -> Value {
    json!({"memberIds": members, "tagNames": tag_names})
}

/// Body of `POST …/memberTags?action=delete`: take people out of tags, by tag id.
pub fn remove_from_tags_body(members: &[String], tag_ids: &[String]) -> Value {
    json!({"memberIds": members, "tagIds": tag_ids})
}

impl Client {
    fn tags_url(team_thread_id: &str, rest: &str) -> String {
        format!("{CSA_V1}/teams/{}/{rest}", enc(team_thread_id))
    }

    /// The tags of a team.
    pub async fn tags(&self, team_thread_id: &str) -> Result<Vec<Tag>> {
        let url = Self::tags_url(
            team_thread_id,
            "tags?tagType=Team&includeTeamTagSettings=true",
        );
        Ok(parse_tags(
            &self.json("tags", Method::GET, &url, None).await?,
        ))
    }

    /// The tags of every team we are in.
    pub async fn all_tags(&self) -> Result<Vec<TeamTags>> {
        let url = format!("{CSA_V1}/teams/users/me/teams/tagCards?tagType=Team&pageSize=500");
        Ok(parse_tag_cards(
            &self.json("tag cards", Method::GET, &url, None).await?,
        ))
    }

    /// Creates a tag with `members` (mris).
    pub async fn create_tag(
        &self,
        team_thread_id: &str,
        name: &str,
        description: &str,
        members: &[String],
    ) -> Result<Tag> {
        let url = Self::tags_url(team_thread_id, "tags");
        let v = self
            .json(
                "create tag",
                Method::POST,
                &url,
                Some(&create_tag_body(name, description, members)),
            )
            .await?;
        parse_tags(&v)
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("tag created but the answer has no tag".into()))
    }

    /// Renames a tag or changes its description. `version` comes from the tag.
    pub async fn edit_tag(
        &self,
        team_thread_id: &str,
        tag: &Tag,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        // Renaming is the one tag call that lives on CSA v2.
        let url = format!(
            "{CSA_V2}/teams/{}/tags/{}",
            enc(team_thread_id),
            enc(&tag.id)
        );
        let version = tag.version.to_string();
        let (status, body) = self
            .request(
                Method::PATCH,
                &url,
                Some(&edit_tag_body(name, description)),
                &[("if-match", version.as_str())],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("edit tag", status, &body));
        }
        Ok(())
    }

    /// Deletes a tag.
    pub async fn delete_tag(&self, team_thread_id: &str, tag_id: &str) -> Result<()> {
        let url = Self::tags_url(team_thread_id, &format!("tags/{}", enc(tag_id)));
        self.json("delete tag", Method::DELETE, &url, None)
            .await
            .map(|_| ())
    }

    /// The members (mris) of a tag.
    pub async fn tag_members(&self, team_thread_id: &str, tag_id: &str) -> Result<Vec<String>> {
        let url = Self::tags_url(
            team_thread_id,
            &format!("tags/{}/members?pageSize=200", enc(tag_id)),
        );
        let v = self.json("tag members", Method::GET, &url, None).await?;
        Ok(parse_tag_members(&v))
    }

    /// Adds people (mris) to tags by tag name.
    pub async fn add_to_tags(
        &self,
        team_thread_id: &str,
        members: &[String],
        tag_names: &[String],
    ) -> Result<()> {
        let url = Self::tags_url(team_thread_id, "memberTags?tagType=Team&action=add");
        self.json(
            "add to tags",
            Method::POST,
            &url,
            Some(&add_to_tags_body(members, tag_names)),
        )
        .await
        .map(|_| ())
    }

    /// Takes people (mris) out of tags by tag id.
    pub async fn remove_from_tags(
        &self,
        team_thread_id: &str,
        members: &[String],
        tag_ids: &[String],
    ) -> Result<()> {
        let url = Self::tags_url(team_thread_id, "memberTags?tagType=Team&action=delete");
        self.json(
            "remove from tags",
            Method::POST,
            &url,
            Some(&remove_from_tags_body(members, tag_ids)),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_json() -> Value {
        json!({"currentMemberIsPartOfTag": true, "memberCount": 2,
            "memberIds": ["8:orgid:a", "8:orgid:b"], "scheduledMembers": [],
            "tagDescription": "d", "tagId": "abc123xyz", "tagName": "Leads",
            "tagType": "team", "version": 3})
    }

    #[test]
    fn parses_a_tag_list_and_a_single_tag() {
        let tags = parse_tags(&json!([tag_json(), {"tagName": "no id"}]));
        assert_eq!(tags.len(), 1);
        let t = &tags[0];
        assert_eq!((t.id.as_str(), t.name.as_str()), ("abc123xyz", "Leads"));
        assert_eq!((t.member_count, t.version, t.is_member), (2, 3, true));
        assert_eq!(t.member_ids, ["8:orgid:a", "8:orgid:b"]);
        assert_eq!(parse_tags(&tag_json()).len(), 1);
        assert!(parse_tags(&json!([])).is_empty());
        assert!(parse_tags(&json!(null)).is_empty());
    }

    #[test]
    fn parses_cross_team_cards() {
        let cards = parse_tag_cards(&json!({"continuationToken": "", "teamTagCards": [
            {"teamId": "19:t@thread.tacv2", "teamName": "T", "tagCards": [tag_json()]},
            {"teamName": "no id", "tagCards": []}
        ]}));
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].team_id, "19:t@thread.tacv2");
        assert_eq!(cards[0].tags[0].name, "Leads");
        assert!(parse_tag_cards(&json!({})).is_empty());
    }

    #[test]
    fn tag_mentions_use_the_tag_mri() {
        let t = parse_tags(&tag_json()).remove(0);
        assert_eq!(t.mri(), "tag:abc123xyz");
        let m = t.mention(4);
        assert_eq!(m.item_id, 4);
        assert_eq!(m.kind, MentionKind::Tag);
        assert_eq!(m.display_name, "Leads");
        assert_eq!(tag_id_of_mri("tag:abc123xyz"), Some("abc123xyz"));
        assert_eq!(tag_id_of_mri("8:orgid:a"), None);
        assert_eq!(tag_id_of_mri("tag:"), None);
        // The mention serialises with the tag type, which is what makes it notify.
        let prop = crate::mentions::mentions_property(&[m]);
        assert!(prop.contains("\"mentionType\":\"tag\""));
        assert!(prop.contains("tag:abc123xyz"));
    }

    #[test]
    fn parses_tag_members() {
        let m = parse_tag_members(&json!({"members": [{"memberId": "8:orgid:a"}, {"x": 1}],
            "totalMembersCount": 1}));
        assert_eq!(m, ["8:orgid:a"]);
        assert!(parse_tag_members(&json!({})).is_empty());
    }

    #[test]
    fn request_bodies() {
        let b = create_tag_body("Leads", "d", &["8:orgid:a".into()]);
        assert_eq!(b["tagName"], "Leads");
        assert_eq!(b["memberIds"][0], "8:orgid:a");
        assert_eq!(b["showInSection"], false);
        assert_eq!(edit_tag_body(Some("n"), None), json!({"tagName": "n"}));
        assert_eq!(
            edit_tag_body(None, Some("d")),
            json!({"tagDescription": "d"})
        );
        assert_eq!(
            add_to_tags_body(&["m".into()], &["Leads".into()]),
            json!({"memberIds": ["m"], "tagNames": ["Leads"]})
        );
        assert_eq!(
            remove_from_tags_body(&["m".into()], &["id1".into()]),
            json!({"memberIds": ["m"], "tagIds": ["id1"]})
        );
    }
}
