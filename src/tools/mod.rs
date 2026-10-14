//! MCP tool surface — 22 tools (16 in read-only mode).
//!
//! All `#[tool]` methods live in the single `#[tool_router]` impl block
//! required by rmcp; business logic lives in the per-domain modules.

pub mod assignments;
pub mod courses;
pub mod files;
pub mod forums;
pub mod grades;
pub mod messaging;
pub mod quizzes;
pub mod raw;
pub mod schedule;
pub mod site;
pub mod users;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::output::{finish_doc, finish_json, finish_json_with_hints};
use crate::tools::schedule::{HINT_COURSE, HINT_TIME};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Empty {}

// ---------- identity / courses ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListCoursesParams {
    /// Filter courses by name (case-insensitive substring); omit for all
    search: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CourseOverviewParams {
    /// The Moodle course id (from list_courses)
    course_id: i64,
}

// ---------- forums ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ForumDiscussionsParams {
    /// The forum id (from get_course_overview)
    forum_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DiscussionPostsParams {
    /// The discussion id (from get_forum_discussions)
    discussion_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct NewDiscussionParams {
    /// The forum id
    forum_id: i64,
    /// Discussion subject
    subject: String,
    /// Discussion message body (plain text or HTML)
    message: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReplyParams {
    /// The post id to reply to (from get_discussion_posts); replying to the
    /// opening post is replying to the thread
    post_id: i64,
    /// Reply subject
    subject: String,
    /// Reply message body (plain text or HTML)
    message: String,
}

// ---------- assignments ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AssignmentIdParams {
    /// The assignment id (from get_course_overview)
    assignment_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SubmitAssignmentParams {
    /// The assignment id
    assignment_id: i64,
    /// Online-text submission content (HTML or plain text)
    text: Option<String>,
    /// Draft item id from upload_file (file submission)
    item_id: Option<i64>,
    /// Finalize the submission for grading. Only needed when the assignment
    /// enables draft submissions (see get_assignment: draft_required);
    /// defaults to false so saving stays reversible
    finalize: Option<bool>,
}

// ---------- files ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct FetchFileParams {
    /// Moodle file URL (from get_course_overview / get_assignment; the token
    /// is injected automatically for pluginfile.php URLs)
    url: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DownloadFileParams {
    /// Moodle file URL (token injected automatically for pluginfile.php)
    url: String,
    /// Local path to write the content to
    save_to: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UploadFileParams {
    /// Local file path to read and upload
    path: String,
    /// File name to store in the draft area; defaults to the file name in path
    filename: Option<String>,
    /// Draft item id; omit to create a new draft area
    item_id: Option<i64>,
}

// ---------- grades ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GradesParams {
    /// The Moodle course id; omit for a cross-course summary of final grades
    course_id: Option<i64>,
}

// ---------- schedule ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ScheduleParams {
    /// Start of the time window as a unix timestamp (seconds); defaults to now
    time_from: Option<i64>,
    /// End of the time window as a unix timestamp (seconds); defaults to now + 30 days
    time_to: Option<i64>,
    /// Restrict to these course ids; omit for all enrolled courses
    course_ids: Option<Vec<i64>>,
}

// ---------- messaging ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct NotificationsParams {
    /// Filter by read state; omit for all
    read: Option<bool>,
    /// Max notifications returned (default 20)
    limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ConversationsParams {
    /// Max conversations returned (default 20)
    limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SendMessageParams {
    /// Recipient user id (from get_conversations members / get_user)
    user_id: i64,
    /// Message text
    text: String,
}

// ---------- users ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetUserParams {
    /// Which field to match: "username" | "email" | "id" | "idnumber"
    field: UserField,
    /// The value of that field
    value: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UserField {
    Username,
    Email,
    Id,
    Idnumber,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchUsersParams {
    /// Search keywords — a name or username fragment. Required: empty or
    /// omitted searches are rejected client-side (they would match every
    /// user on the site and time out)
    search: Option<String>,
    /// Max users returned (default 20)
    limit: Option<u32>,
}

// ---------- quizzes ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct QuizAttemptsParams {
    /// The quiz id (from get_course_overview)
    quiz_id: i64,
}

// ---------- escape hatch ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RawCallParams {
    /// The Moodle WS function name (see list_functions)
    wsfunction: String,
    /// Function parameters as a JSON object; nested arrays/objects are
    /// flattened to Moodle's PHP bracket notation
    params: Option<Value>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListFunctionsParams {
    /// Keyword to filter function names (case-insensitive substring)
    query: String,
}

/// Cache entries expire after this long: enrollment changes and function
/// toggles made during a session become visible without restarting.
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// A cached value with its fill time (see [`CACHE_TTL`]).
type Cached<T> = Arc<RwLock<Option<(std::time::Instant, T)>>>;

/// The MCP server: 22 tools for a student's Moodle (16 when read-only
/// mode filters the write/upload/escape-hatch tools out of the router).
#[derive(Clone)]
pub struct MoodleMcp {
    client: Arc<MoodleClient>,
    site_cache: Cached<Value>,
    courses_cache: Cached<Vec<i64>>,
    tool_router: ToolRouter<Self>,
    read_only: bool,
}

/// Tools hidden in read-only mode (MOODLE_READ_ONLY=1): the four WRITE
/// operations, file upload (a read-only agent has no business staging files
/// on the server), and the raw escape hatch — `call_ws_function` can invoke
/// write functions directly and would bypass the restriction.
pub(crate) const READ_ONLY_DISABLED_TOOLS: &[&str] = &[
    "create_forum_discussion",
    "reply_to_discussion",
    "submit_assignment",
    "send_message",
    "upload_file",
    "call_ws_function",
];

impl MoodleMcp {
    /// Cached `core_webservice_get_site_info` payload (identity, functions,
    /// release — the bootstrap record), refreshed every 10 minutes.
    async fn site_info_cached(&self) -> Result<Value, MoodleError> {
        if let Some((at, hit)) = self.site_cache.read().await.clone()
            && at.elapsed() < CACHE_TTL
        {
            return Ok(hit);
        }
        let info: Value = self
            .client
            .call("core_webservice_get_site_info", &json!({}))
            .await?;
        *self.site_cache.write().await = Some((std::time::Instant::now(), info.clone()));
        Ok(info)
    }

    /// The token owner's user id.
    async fn current_user_id(&self) -> Result<i64, MoodleError> {
        Ok(self
            .site_info_cached()
            .await?
            .get("userid")
            .and_then(Value::as_i64)
            .unwrap_or(0))
    }

    /// The list of WS functions enabled for this token. Entries are objects
    /// `{name, version}` on every Moodle version; plain strings are tolerated.
    async fn allowed_functions(&self) -> Vec<String> {
        self.site_info_cached()
            .await
            .ok()
            .and_then(|v| v.get("functions").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| {
                if let Some(s) = f.as_str() {
                    Some(s.to_string())
                } else {
                    f.get("name").and_then(Value::as_str).map(str::to_string)
                }
            })
            .collect()
    }

    /// Resolve the WS function name for version-dependent features: the first
    /// candidate enabled for this token (optimistically the first candidate
    /// when the list is unavailable).
    async fn pick_function(&self, candidates: &[&str]) -> Result<String, MoodleError> {
        let allowed = self.allowed_functions().await;
        if allowed.is_empty() {
            return Ok(candidates[0].to_string());
        }
        candidates
            .iter()
            .find(|c| allowed.iter().any(|f| f == *c))
            .map(|c| (*c).to_string())
            .ok_or_else(|| {
                MoodleError::Config(format!(
                    "none of the required functions are enabled for this token: {}",
                    candidates.join(", ")
                ))
            })
    }

    /// The token owner's enrolled course ids (cached, 10-minute TTL).
    async fn enrolled_course_ids(&self) -> Result<Vec<i64>, MoodleError> {
        if let Some((at, hit)) = self.courses_cache.read().await.clone()
            && at.elapsed() < CACHE_TTL
        {
            return Ok(hit);
        }
        let user_id = self.current_user_id().await?;
        let raw: Value = self
            .client
            .call(
                "core_enrol_get_users_courses",
                &json!({ "userid": user_id }),
            )
            .await?;
        let ids: Vec<i64> = raw
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| c.get("id").and_then(Value::as_i64))
            .collect();
        *self.courses_cache.write().await = Some((std::time::Instant::now(), ids.clone()));
        Ok(ids)
    }
}

#[tool_router]
impl MoodleMcp {
    /// Build the server, optionally filtering the tool surface. Read-only
    /// mode hides the write/upload/escape-hatch tools
    /// ([READ_ONLY_DISABLED_TOOLS]); `disabled_tools` (MOODLE_DISABLE_TOOLS)
    /// drops additional names on top — unknown names are warned about, not
    /// fatal. Disabled tools vanish from `tools/list` and calls to them are
    /// rejected by the router, so clients see the real capability surface.
    pub fn new_with_policy(
        client: Arc<MoodleClient>,
        read_only: bool,
        disabled_tools: Vec<String>,
    ) -> Self {
        let mut tool_router = Self::tool_router();
        let mut names = disabled_tools;
        if read_only {
            names.extend(READ_ONLY_DISABLED_TOOLS.iter().map(|s| (*s).to_string()));
        }
        for name in names {
            if !tool_router.has_route(&name) {
                tracing::warn!(
                    tool = %name,
                    "MOODLE_DISABLE_TOOLS names a tool this server does not expose — ignored"
                );
                continue;
            }
            tool_router.disable_route(name);
        }
        Self {
            client,
            site_cache: Arc::new(RwLock::new(None)),
            courses_cache: Arc::new(RwLock::new(None)),
            tool_router,
            read_only,
        }
    }

    // ---------- identity ----------

    #[tool(
        description = "Who the token belongs to (user_id, username, fullname) and which Moodle site/release it is connected to. Also an implicit connectivity check: if this fails, every other tool will fail too."
    )]
    async fn whoami(&self, Parameters(_): Parameters<Empty>) -> Result<CallToolResult, McpError> {
        let res = self
            .site_info_cached()
            .await
            .map(|v| site::whoami_shape(&v));
        finish(res)
    }

    // ---------- courses ----------

    #[tool(
        description = "List the courses you are enrolled in (course_id, name, dates, progress). Optionally filter by name."
    )]
    async fn list_courses(
        &self,
        Parameters(p): Parameters<ListCoursesParams>,
    ) -> Result<CallToolResult, McpError> {
        let search = p.search;
        finish(match self.current_user_id().await {
            Ok(uid) => courses::list_courses(&self.client, uid, search.as_deref()).await,
            Err(e) => Err(e),
        })
    }

    #[tool(
        description = "The full map of one course as a markdown document: assignments, quizzes, forums, graded items, and every section with its files (download URLs included). The entry point for anything about a specific course."
    )]
    async fn get_course_overview(
        &self,
        Parameters(p): Parameters<CourseOverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        let user_id = match self.current_user_id().await {
            Ok(uid) => uid,
            Err(e) => return finish(Err(e)),
        };
        match courses::get_course_overview(&self.client, p.course_id, user_id).await {
            Ok((md, structured)) => finish_doc(md, structured),
            Err(e) => finish(Err(e)),
        }
    }

    // ---------- forums ----------

    #[tool(
        description = "List the discussions in one forum (subject, preview of the opening post, author, replies, pinned/locked). forum_id comes from get_course_overview."
    )]
    async fn get_forum_discussions(
        &self,
        Parameters(p): Parameters<ForumDiscussionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let wsfunction = self
            .pick_function(&[
                "mod_forum_get_discussions",
                "mod_forum_get_forum_discussions",
                "mod_forum_get_forum_discussions_paginated",
            ])
            .await;
        let res = match wsfunction {
            Ok(f) => forums::get_forum_discussions(&self.client, &f, p.forum_id).await,
            Err(e) => Err(e),
        };
        finish(res)
    }

    #[tool(
        description = "Read a whole discussion thread: the opening post and every reply in order, with author, timestamps, converted message text and post attachments."
    )]
    async fn get_discussion_posts(
        &self,
        Parameters(p): Parameters<DiscussionPostsParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(forums::get_discussion_posts(&self.client, p.discussion_id).await)
    }

    #[tool(
        description = "Start a new discussion in a forum. WRITE operation — confirm with the user before calling."
    )]
    async fn create_forum_discussion(
        &self,
        Parameters(p): Parameters<NewDiscussionParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            forums::create_forum_discussion(&self.client, p.forum_id, &p.subject, &p.message, None)
                .await,
        )
    }

    #[tool(
        description = "Reply to a forum post (replying to the opening post = replying to the thread). WRITE operation — confirm with the user before calling."
    )]
    async fn reply_to_discussion(
        &self,
        Parameters(p): Parameters<ReplyParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(forums::reply_to_discussion(&self.client, p.post_id, &p.subject, &p.message).await)
    }

    // ---------- assignments ----------

    #[tool(
        description = "Everything about one assignment as a markdown document: times, draft/attempt state, full description, attachment files (with URLs), your submission (text/files), grade and teacher feedback. assignment_id comes from get_course_overview."
    )]
    async fn get_assignment(
        &self,
        Parameters(p): Parameters<AssignmentIdParams>,
    ) -> Result<CallToolResult, McpError> {
        match assignments::get_assignment(&self.client, p.assignment_id).await {
            Ok((md, structured)) => finish_doc(md, structured),
            Err(e) => Ok(crate::error::moodle_to_tool_result(e)),
        }
    }

    #[tool(
        description = "Submit an assignment: online text and/or draft files (item_id from upload_file), optionally finalizing for grading. finalize is only needed when the assignment enables draft submissions (see get_assignment). WRITE operation — confirm with the user before calling."
    )]
    async fn submit_assignment(
        &self,
        Parameters(p): Parameters<SubmitAssignmentParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            assignments::submit_assignment(
                &self.client,
                p.assignment_id,
                p.text.as_deref(),
                p.item_id,
                p.finalize.unwrap_or(false),
            )
            .await,
        )
    }

    // ---------- grades ----------

    #[tool(
        description = "Grades. Without course_id: a cross-course summary of final grades. With course_id: every grade item of that course (quizzes, manual items, course total) with ranges, weights and teacher feedback."
    )]
    async fn get_grades(
        &self,
        Parameters(p): Parameters<GradesParams>,
    ) -> Result<CallToolResult, McpError> {
        let res = match self.current_user_id().await {
            Ok(uid) => match grades::get_grades(&self.client, p.course_id, uid).await {
                // 3.7's overview API omits course names — fill them from the roster.
                Ok(v) if v["mode"] == json!("overview") => {
                    grades::enrich_overview_course_names(&self.client, v, uid).await
                }
                other => other,
            },
            Err(e) => Err(e),
        };
        finish(res)
    }

    // ---------- schedule ----------

    #[tool(
        description = "Your timeline: assignment due dates, quiz closes and calendar events in one merged list, sorted by time. Defaults to the next 30 days across all enrolled courses."
    )]
    async fn get_schedule(
        &self,
        Parameters(p): Parameters<ScheduleParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut hints = Vec::new();
        if p.time_from.is_none() || p.time_to.is_none() {
            hints.push(HINT_TIME.to_string());
        }
        if p.course_ids.is_none() {
            hints.push(HINT_COURSE.to_string());
        }
        // Agent-provided course_ids win; omitted → all enrolled courses.
        let ids = match p.course_ids.clone() {
            Some(ids) if !ids.is_empty() => ids,
            _ => match self.enrolled_course_ids().await {
                Ok(ids) => ids,
                Err(e) => return Ok(crate::error::moodle_to_tool_result(e)),
            },
        };
        match schedule::get_schedule(&self.client, &ids, p.time_from, p.time_to).await {
            Ok((v, extra_hints)) => {
                let mut all = hints.clone();
                all.extend(extra_hints);
                finish_json_with_hints(v, &all)
            }
            Err(e) => finish(Err(e)),
        }
    }

    // ---------- messaging ----------

    #[tool(
        description = "Your recent notifications (announcements, grade releases, teacher messages)."
    )]
    async fn get_notifications(
        &self,
        Parameters(p): Parameters<NotificationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let wsfunction = self
            .pick_function(&[
                "core_message_get_notifications",
                "core_message_get_messages",
                // Last resort only (different response shape; handled via
                // the `notifications` key below) — keeps verified paths first.
                "message_popup_get_popup_notifications",
            ])
            .await;
        // read=2 ("both") exists since Moodle 4.0; up to 3.11 the param is
        // a boolean and "omit for all" needs two merged calls.
        let read_both_supported = self
            .site_info_cached()
            .await
            .ok()
            .and_then(|i| {
                i.get("release")
                    .and_then(Value::as_str)
                    .map(crate::util::release_major)
            })
            .flatten()
            .is_some_and(|major| major >= 4);
        let res = match wsfunction {
            Ok(f) => match self.current_user_id().await {
                Ok(uid) => {
                    messaging::get_notifications(
                        &self.client,
                        &f,
                        uid,
                        p.read,
                        p.limit.unwrap_or(20),
                        read_both_supported,
                    )
                    .await
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        finish(res)
    }

    #[tool(
        description = "Your messaging conversations (display name, unread state, last message, member ids for send_message)."
    )]
    async fn get_conversations(
        &self,
        Parameters(p): Parameters<ConversationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let limit = p.limit.unwrap_or(20);
        let res = match self.current_user_id().await {
            Ok(uid) => messaging::get_conversations(&self.client, uid, limit).await,
            Err(e) => Err(e),
        };
        finish(res)
    }

    #[tool(
        description = "Send a direct message to a user. WRITE operation — the message is sent as you; draft it and confirm with the user before calling."
    )]
    async fn send_message(
        &self,
        Parameters(p): Parameters<SendMessageParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(messaging::send_message(&self.client, p.user_id, &p.text).await)
    }

    // ---------- users ----------

    #[tool(
        description = "Look up one user's profile by an exact id, username, email or idnumber (resolves ids found in conversations, notifications and forum posts into names/emails). Whether other students' profiles are visible is the site's privacy policy."
    )]
    async fn get_user(
        &self,
        Parameters(p): Parameters<GetUserParams>,
    ) -> Result<CallToolResult, McpError> {
        let field = match p.field {
            UserField::Username => "username",
            UserField::Email => "email",
            UserField::Id => "id",
            UserField::Idnumber => "idnumber",
        };
        finish(users::get_user_by_field(&self.client, field, &p.value).await)
    }

    #[tool(
        description = "Search users by keyword. May be permission-denied for student accounts (the error will say so)."
    )]
    async fn search_users(
        &self,
        Parameters(p): Parameters<SearchUsersParams>,
    ) -> Result<CallToolResult, McpError> {
        let wsfunction = self
            .pick_function(&["core_user_get_users", "core_message_message_search_users"])
            .await;
        let res = match wsfunction {
            Ok(f) => match self.current_user_id().await {
                Ok(uid) => {
                    users::search_users(
                        &self.client,
                        &f,
                        uid,
                        p.search.as_deref(),
                        p.limit.unwrap_or(20),
                    )
                    .await
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        finish(res)
    }

    // ---------- quizzes (read-only) ----------

    #[tool(
        description = "Your attempts on a quiz: state, start/finish times, total marks. quiz_id comes from get_course_overview. Quiz-taking itself is intentionally not provided."
    )]
    async fn get_quiz_attempts(
        &self,
        Parameters(p): Parameters<QuizAttemptsParams>,
    ) -> Result<CallToolResult, McpError> {
        let res = match self.current_user_id().await {
            Ok(uid) => quizzes::get_quiz_attempts(&self.client, p.quiz_id, uid).await,
            Err(e) => Err(e),
        };
        finish(res)
    }

    // ---------- files ----------

    #[tool(
        description = "Fetch a Moodle file and return its content as markdown. PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV are converted with anydoc; plain text passes through. For non-convertible files (zip, images) use download_file instead. The web service token is injected automatically for pluginfile.php URLs."
    )]
    async fn fetch_file(
        &self,
        Parameters(p): Parameters<FetchFileParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(files::fetch_file(&self.client, &p.url).await)
    }

    #[tool(
        description = "Download a Moodle file to a local path (metadata only — binary payloads never enter the conversation). The web service token is injected automatically for pluginfile.php URLs."
    )]
    async fn download_file(
        &self,
        Parameters(p): Parameters<DownloadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(files::download_file(&self.client, &p.url, &p.save_to).await)
    }

    #[tool(
        description = "Upload a local file to the user's draft file area; returns the draft item_id referencing the uploaded file. Multiple files: call repeatedly with the same item_id."
    )]
    async fn upload_file(
        &self,
        Parameters(p): Parameters<UploadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let item_id = p.item_id.unwrap_or_else(crate::util::unique_item_id);
        finish(
            files::upload_file_from_path(&self.client, &p.path, p.filename.as_deref(), item_id)
                .await,
        )
    }

    // ---------- escape hatch ----------

    #[tool(
        description = "Call any Moodle web service function by name with free-form JSON parameters (advanced escape hatch). Nested arrays/objects are flattened to Moodle's PHP bracket notation. The function must be enabled for this token — discover candidates with list_functions. WARNING: destructive functions (delete/block/reset) take effect immediately without confirmation — double-check with the user first."
    )]
    async fn call_ws_function(
        &self,
        Parameters(p): Parameters<RawCallParams>,
    ) -> Result<CallToolResult, McpError> {
        let allowed = self.allowed_functions().await;
        // LLM clients sometimes serialize `params` as a JSON-encoded string.
        // Accept both an object and a string containing a JSON object, but
        // fail loudly (never silently {}) so a malformed call cannot invoke
        // the function with unintended defaults.
        let params = match p.params {
            Some(Value::String(s)) => match serde_json::from_str::<Value>(&s) {
                Ok(v) if v.is_object() => v,
                Ok(_) | Err(_) => {
                    return Ok(crate::error::moodle_to_tool_result(MoodleError::Config(
                        "params must be a JSON object (or a string containing one) — \
                         fix the arguments and retry"
                            .into(),
                    )));
                }
            },
            Some(v) => v,
            None => serde_json::json!({}),
        };
        finish(raw::call_ws_function(&self.client, &p.wsfunction, &params, &allowed).await)
    }

    #[tool(
        description = "Search the Moodle WS functions this token may call (case-insensitive substring, e.g. 'wiki', 'forum', 'assign'). Matches can then be executed via call_ws_function."
    )]
    async fn list_functions(
        &self,
        Parameters(p): Parameters<ListFunctionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let allowed = self.allowed_functions().await;
        finish(site::list_functions(&allowed, &p.query))
    }
}

#[tool_handler(router = self.tool_router.clone())]
impl ServerHandler for MoodleMcp {
    fn get_info(&self) -> ServerConfig {
        let mut instructions =
            "Moodle MCP server: read and operate a Moodle site through its web services \
             as the token owner. Navigation: list_courses → get_course_overview → \
             per-item tools. All write operations (assignment submissions, forum posts, \
             messages) act as the token owner — confirm intent with the user before \
             submitting or sending."
                .to_string();
        if self.read_only {
            instructions.push_str(
                " Read-only mode is active: write, upload and raw-call tools are disabled.",
            );
        }
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("moodle-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(instructions)
    }
}

/// Map a domain result onto a tool result. Moodle failures become
/// tool-level errors (`isError: true`) so the model can read the message and
/// self-correct; only serialization failures become protocol errors.
fn finish(result: Result<Value, MoodleError>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(v) => finish_json(v),
        Err(e) => Ok(crate::error::moodle_to_tool_result(e)),
    }
}
