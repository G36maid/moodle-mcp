use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{strip_html, truncate_chars, unix_to_iso};

/// `mod_forum_get_forums_by_courses` — forums in the given courses
/// (all enrolled courses when the list is empty).
pub async fn get_forums(client: &MoodleClient, course_ids: &[i64]) -> Result<Value, MoodleError> {
    let params = if course_ids.is_empty() {
        json!({})
    } else {
        json!({ "courseids": course_ids })
    };
    let v: Value = client.call("mod_forum_get_forums_by_courses", &params).await?;
    let forums: Vec<Value> = v
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            json!({
                "forum_id": f.get("id").cloned().unwrap_or(Value::Null),
                "course_id": f.get("course").cloned().unwrap_or(Value::Null),
                "name": f.get("name").cloned().unwrap_or(Value::Null),
                "type": f.get("type").cloned().unwrap_or(Value::Null),
                "intro": f.get("intro").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 400)),
                "due": unix_to_iso(f.get("duedate").cloned().unwrap_or(Value::Null)),
                "locked_at": unix_to_iso(f.get("locktime").cloned().unwrap_or(Value::Null)),
                "maxattachments": f.get("maxattachments").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "count": forums.len(), "forums": forums }))
}

/// List discussions in a forum. `wsfunction` varies by Moodle version:
/// `mod_forum_get_discussions` (modern) or
/// `mod_forum_get_forum_discussions(_paginated)` (Moodle 3.x).
///
/// The modern function accepts `sortby`; the 3.x variants reject ANY sortby
/// value and rely on server defaults (timemodified DESC) — so sort params are
/// only sent for the modern one.
pub async fn get_forum_discussions(
    client: &MoodleClient,
    wsfunction: &str,
    forum_id: i64,
    page: u32,
    per_page: u32,
) -> Result<Value, MoodleError> {
    let mut params = json!({
        "forumid": forum_id,
        "page": page,
        "perpage": per_page
    });
    if wsfunction == "mod_forum_get_discussions" {
        params["sortby"] = json!("modified");
        params["sortdirection"] = json!("DESC");
    }
    let v: Value = client.call(wsfunction, &params).await?;
    // Modern `mod_forum_get_discussions` returns a top-level array; the 3.x
    // variants wrap it in `{"discussions": [...]}`. Normalize both.
    let raw = if v.is_array() {
        v
    } else {
        v.get("discussions").cloned().unwrap_or_else(|| json!([]))
    };
    let discussions: Vec<Value> = raw
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|d| {
            json!({
                "discussion_id": d.get("id").cloned().unwrap_or(Value::Null),
                "subject": crate::util::first_str(&d, &["name", "subject"])
                    .map(|s| Value::String(s.to_string()))
                    .unwrap_or(Value::Null),
                "author": d.get("userfullname").cloned().unwrap_or(Value::Null),
                "author_id": d.get("userid").cloned().unwrap_or(Value::Null),
                "created": unix_to_iso(d.get("created").cloned().unwrap_or(Value::Null)),
                "last_activity": unix_to_iso(d.get("modified").cloned().unwrap_or(Value::Null)),
                "numreplies": d.get("numreplies").cloned().unwrap_or(Value::Null),
                "numposts": d.get("numposts").cloned().unwrap_or(Value::Null),
                "pinned": d.get("pinned").cloned().unwrap_or(Value::Null),
                "locked": d.get("discussionlocked").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "forum_id": forum_id, "count": discussions.len(), "discussions": discussions }))
}

/// `mod_forum_get_discussion_posts` — all posts in a discussion thread.
///
/// Modern Moodle accepts `page`/`perpage`; older versions (e.g. 3.7) reject
/// them with `invalidparameter`, so on that error we retry without pagination.
pub async fn get_discussion_posts(
    client: &MoodleClient,
    discussion_id: i64,
    page: u32,
    per_page: u32,
) -> Result<Value, MoodleError> {
    let with_paging = json!({
        "discussionid": discussion_id,
        "sortby": "created",
        "sortdirection": "ASC",
        "page": page,
        "perpage": per_page
    });
    let fallback = json!({
        "discussionid": discussion_id,
        "sortby": "created",
        "sortdirection": "ASC"
    });
    let v: Value = match client.call("mod_forum_get_discussion_posts", &with_paging).await {
        Err(MoodleError::Api { errorcode, .. }) if errorcode == "invalidparameter" => {
            client.call("mod_forum_get_discussion_posts", &fallback).await?
        }
        other => other?,
    };
    let posts: Vec<Value> = v
        .get("posts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            json!({
                "post_id": p.get("id").cloned().unwrap_or(Value::Null),
                "parent_post_id": p.get("parentid").cloned().unwrap_or(Value::Null),
                "subject": p.get("subject").cloned().unwrap_or(Value::Null),
                "author": crate::util::dig(&p, "author.fullname").cloned().unwrap_or(Value::Null),
                "author_id": crate::util::dig(&p, "author.id").cloned().unwrap_or(Value::Null),
                "created": unix_to_iso(p.get("timecreated").cloned().unwrap_or(Value::Null)),
                "message": p.get("message").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 2000)),
                "unread": p.get("unread").cloned().unwrap_or(Value::Null),
                "isprivatereply": p.get("isprivatereply").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({
        "discussion_id": discussion_id,
        "count": posts.len(),
        "posts": posts
    }))
}

/// `mod_forum_add_discussion` — start a new discussion in a forum.
pub async fn create_forum_discussion(
    client: &MoodleClient,
    forum_id: i64,
    subject: &str,
    message: &str,
    group_id: Option<i64>,
) -> Result<Value, MoodleError> {
    let mut params = json!({
        "forumid": forum_id,
        "subject": subject,
        "message": message
    });
    if let Some(g) = group_id {
        params["groupid"] = json!(g);
    }
    let v: Value = client.call("mod_forum_add_discussion", &params).await?;
    Ok(json!({
        "created": true,
        "discussion_id": v.get("discussionid").cloned().unwrap_or(Value::Null),
        "warnings": v.get("warnings").cloned().unwrap_or(json!([]))
    }))
}

/// `mod_forum_add_discussion_post` — reply to an existing post.
pub async fn reply_to_discussion(
    client: &MoodleClient,
    post_id: i64,
    subject: &str,
    message: &str,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "mod_forum_add_discussion_post",
            &json!({
                "postid": post_id,
                "subject": subject,
                "message": message
            }),
        )
        .await?;
    Ok(json!({
        "created": true,
        "post_id": v.get("postid").cloned().unwrap_or(Value::Null),
        "warnings": v.get("warnings").cloned().unwrap_or(json!([]))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_discussion_subject_fallback() {
        // explicit null falls back to the next key
        let d = json!({"id": 1, "name": null, "subject": "Hello", "userfullname": "Ada"});
        let subject = crate::util::first_str(&d, &["name", "subject"]);
        assert_eq!(subject, Some("Hello"));
        // absent keys fall back too
        let d2 = json!({"id": 2, "subject": "Other"});
        assert_eq!(crate::util::first_str(&d2, &["name", "subject"]), Some("Other"));
        assert_eq!(crate::util::first_str(&d2, &["name"]), None);
    }
}
