//! Forums: a strict top-down hierarchy —
//! course map (forum ids) → discussions → posts, plus the two write actions.

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{decode_entities, first_str, html_to_markdown, truncate_chars, unix_to_iso};

/// `get_forum_discussions` — discussions in a forum.
///
/// The WS function name varies by Moodle version (3.7 lacks
/// `mod_forum_get_discussions`); `wsfunction` is resolved by the caller via
/// `pick_function`. The 3.x variants reject `sortby` and wrap the list in
/// `{"discussions": [...]}` — both handled here.
pub async fn get_forum_discussions(
    client: &MoodleClient,
    wsfunction: &str,
    forum_id: i64,
) -> Result<Value, MoodleError> {
    let mut params = json!({ "forumid": forum_id });
    if wsfunction == "mod_forum_get_discussions" {
        params["sortby"] = json!("modified");
        params["sortdirection"] = json!("DESC");
    }
    let v: Value = client.call(wsfunction, &params).await?;

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
            // Legacy (3.x) records are the opening POST: `id` = post id,
            // `discussion` = discussion id. Modern summaries: `id` = discussion
            // id. Resolve accordingly and expose the opening post id for
            // reply_to_discussion.
            let discussion_id = d
                .get("discussion")
                .cloned()
                .unwrap_or_else(|| d.get("id").cloned().unwrap_or(Value::Null));
            let subject =
                decode_entities(first_str(&d, &["name", "subject"]).unwrap_or_default());
            let preview = truncate_chars(
                &html_to_markdown(d.get("message").and_then(Value::as_str).unwrap_or_default()),
                200,
            );
            json!({
                "discussion_id": discussion_id,
                "opening_post_id": d.get("id").cloned().unwrap_or(Value::Null),
                "subject": subject,
                "preview": preview,
                "author": first_str(&d, &["userfullname", "usermodifiedfullname"])
                    .map(Value::from).unwrap_or(Value::Null),
                "author_id": d.get("userid").cloned().unwrap_or(Value::Null),
                "created": unix_to_iso(d.get("created").cloned().unwrap_or(Value::Null)),
                "last_activity": unix_to_iso(
                    d.get("modified").or_else(|| d.get("timemodified")).cloned()
                        .unwrap_or(Value::Null)),
                "numreplies": d.get("numreplies").cloned().unwrap_or(Value::Null),
                "pinned": d.get("pinned").cloned().unwrap_or(Value::Null),
                "locked": d.get("locked").cloned().unwrap_or(Value::Null),
                "has_attachment": d.get("attachment").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({
        "forum_id": forum_id,
        "count": discussions.len(),
        "discussions": discussions
    }))
}

/// `mod_forum_get_discussion_posts` — every post in a thread (opening post
/// included), oldest first. Older Moodle versions (e.g. 3.7) reject the
/// pagination params; on `invalidparameter` we transparently retry without
/// them.
pub async fn get_discussion_posts(
    client: &MoodleClient,
    discussion_id: i64,
) -> Result<Value, MoodleError> {
    let with_paging = json!({ "discussionid": discussion_id });
    let v: Value = match client
        .call("mod_forum_get_discussion_posts", &with_paging)
        .await
    {
        Err(MoodleError::Api { errorcode, .. }) if errorcode == "invalidparameter" => {
            client
                .call(
                    "mod_forum_get_discussion_posts",
                    &json!({ "discussionid": discussion_id }),
                )
                .await?
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
            let attachments: Vec<Value> = p
                .get("attachments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| {
                    json!({
                        "filename": a.get("filename").cloned().unwrap_or(Value::Null),
                        "fileurl": a.get("fileurl").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect();
            json!({
                "post_id": p.get("id").cloned().unwrap_or(Value::Null),
                "parent_post_id": p.get("parentid").cloned().unwrap_or(Value::Null),
                "subject": decode_entities(
                    p.get("subject").and_then(Value::as_str).unwrap_or_default()),
                "author": crate::util::dig(&p, "author.fullname").cloned().unwrap_or(Value::Null),
                "author_id": crate::util::dig(&p, "author.id").cloned().unwrap_or(Value::Null),
                "created": unix_to_iso(p.get("timecreated").cloned().unwrap_or(Value::Null)),
                "message": p.get("message").and_then(Value::as_str)
                    .map(html_to_markdown),
                "attachments": attachments,
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
        "forum_id": forum_id,
        "discussion_id": v.get("discussionid").cloned().unwrap_or(Value::Null),
    }))
}

/// `mod_forum_add_discussion_post` — reply to an existing post. Replying to
/// the thread's opening post is the same operation (its id is the post id).
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
    }))
}
