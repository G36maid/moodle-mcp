//! MCP tool surface. All `#[tool]` methods live in the single
//! `#[tool_router]` impl block required by rmcp; business logic is delegated
//! to the per-domain modules.

pub mod assignments;
pub mod calendar;
pub mod courses;
pub mod files;
pub mod forums;
pub mod grades;
pub mod messaging;
pub mod quizzes;
pub mod raw;
pub mod site;
pub mod users;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::client::MoodleClient;
use crate::error::{MoodleError, moodle_to_tool_result};
use crate::util::strip_empty_warnings;

const MAX_OUTPUT_CHARS: usize = 250_000;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Empty {}  // tools without parameters

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListCoursesParams {
    /// User id; defaults to the token owner (from site_info)
    user_id: Option<i64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CourseContentsParams {
    /// The Moodle course id
    course_id: i64,
    /// Filter to a single section by its id
    section_id: Option<i64>,
    /// Filter to a single section by its number (0-based)
    section_number: Option<i64>,
    /// Omit module (activity) information
    #[serde(default)]
    exclude_modules: bool,
    /// Omit file/content lists
    #[serde(default)]
    exclude_contents: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchCoursesParams {
    /// Search keywords
    search: String,
    /// Max results (default 20)
    limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AssignmentsParams {
    /// Restrict to these course ids; omit for all enrolled courses
    course_ids: Option<Vec<i64>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AssignmentIdParams {
    /// The assignment id (from get_assignments)
    assignment_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SubmitTextParams {
    /// The assignment id
    assignment_id: i64,
    /// The text to submit as the online-text submission
    text: String,
    /// Text format: "html" (default), "markdown", or "plain"
    format: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UploadFileParams {
    /// File name to store in the draft area
    filename: String,
    /// File content, base64-encoded
    content_base64: String,
    /// Draft item id; omit to create a new draft area (recommended)
    item_id: Option<i64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SubmitFilesParams {
    /// The assignment id
    assignment_id: i64,
    /// Draft item id returned by upload_file
    item_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SubmitForGradingParams {
    /// The assignment id
    assignment_id: i64,
    /// Accept the submission statement (default true; required when the
    /// assignment demands one)
    accept_submission_statement: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GradesParams {
    /// The Moodle course id
    course_id: i64,
    /// User id; defaults to the token owner
    user_id: Option<i64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CalendarParams {
    /// Restrict to these course ids; omit for all
    course_ids: Option<Vec<i64>>,
    /// Only events starting at/after this unix timestamp (seconds)
    time_from: Option<i64>,
    /// Only events starting at/before this unix timestamp (seconds)
    time_to: Option<i64>,
    /// Max events returned (default 50)
    limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DeadlinesParams {
    /// Look this many days ahead (default 30)
    days_ahead: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ForumsParams {
    /// Restrict to these course ids; omit for all enrolled courses
    course_ids: Option<Vec<i64>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DiscussionsParams {
    /// The forum id (from get_forums)
    forum_id: i64,
    /// Zero-based page (default 0)
    page: Option<u32>,
    /// Page size (default 20)
    per_page: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PostsParams {
    /// The discussion id (from get_forum_discussions)
    discussion_id: i64,
    /// Zero-based page (default 0)
    page: Option<u32>,
    /// Page size (default 30)
    per_page: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct NewDiscussionParams {
    /// The forum id
    forum_id: i64,
    /// Discussion subject
    subject: String,
    /// Discussion message body (plain text or HTML)
    message: String,
    /// Restrict to a group id (group discussions only)
    group_id: Option<i64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReplyParams {
    /// The post id to reply to (from get_discussion_posts)
    post_id: i64,
    /// Reply subject
    subject: String,
    /// Reply message body (plain text or HTML)
    message: String,
}

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
    /// Recipient user id
    user_id: i64,
    /// Message text
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchUsersParams {
    /// Search keywords (name/username); omit to list users
    search: Option<String>,
    /// Max users returned (default 20)
    limit: Option<u32>,
}

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
pub struct QuizzesParams {
    /// Restrict to these course ids; omit for all enrolled courses
    course_ids: Option<Vec<i64>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct QuizAttemptsParams {
    /// The quiz id (from get_quizzes)
    quiz_id: i64,
    /// User id; defaults to the token owner
    user_id: Option<i64>,
    /// "all" (default) | "finished" | "incomplete"
    status: Option<QuizStatus>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum QuizStatus {
    All,
    Finished,
    Incomplete,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DownloadFileParams {
    /// Moodle file URL (e.g. the fileurl from get_course_contents; the token
    /// is injected automatically for pluginfile.php URLs)
    url: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RawCallParams {
    /// The Moodle WS function name, e.g. "core_webservice_get_site_info"
    wsfunction: String,
    /// Function parameters as a JSON object; arrays/objects are flattened to
    /// Moodle's PHP bracket notation (e.g. {"courseids":[2]} -> courseids[0]=2)
    params: Option<Value>,
}

/// The MCP server: tools for a student's daily Moodle workflow.
#[derive(Clone)]
pub struct MoodleMcp {
    client: Arc<MoodleClient>,
    site_cache: Arc<RwLock<Option<Value>>>,
    tool_router: ToolRouter<Self>,
}

impl MoodleMcp {
    pub fn new(client: Arc<MoodleClient>) -> Self {
        Self {
            client,
            site_cache: Arc::new(RwLock::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    /// Cached `core_webservice_get_site_info` payload.
    async fn site_info_cached(&self) -> Result<Value, MoodleError> {
        if let Some(hit) = self.site_cache.read().await.clone() {
            return Ok(hit);
        }
        let info = site::run(&self.client).await?;
        *self.site_cache.write().await = Some(info.clone());
        Ok(info)
    }

    /// The token owner's user id (from the cached site info).
    async fn current_user_id(&self) -> Result<i64, MoodleError> {
        Ok(self
            .site_info_cached()
            .await?
            .get("userid")
            .and_then(Value::as_i64)
            .unwrap_or(0))
    }

    /// The list of WS functions enabled for this token.
    ///
    /// `site_info.functions` entries are objects `{name, version}` on every
    /// Moodle version; tolerate plain strings for robustness.
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

    /// Pick the first WS function from `candidates` that is enabled for this
    /// token (Moodle versions expose different names for the same feature,
    /// e.g. Moodle 3.7 lacks `mod_forum_get_discussions` but has
    /// `mod_forum_get_forum_discussions`). If the function list could not be
    /// determined, optimistically return the first candidate.
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

    /// Resolve the effective user id (explicit param, else the token owner)
    /// and run `f` with it. Errors flow through the tool-level error path.
    async fn with_user_id(
        &self,
        user_id: Option<i64>,
        f: impl AsyncFnOnce(i64) -> Result<Value, MoodleError>,
    ) -> Result<Value, MoodleError> {
        let uid = match user_id {
            Some(id) => id,
            None => self.current_user_id().await?,
        };
        f(uid).await
    }
}

#[tool_router]
impl MoodleMcp {
    // ---------- site ----------

    /// Identity, connection check and capability probe: returns the site name,
    /// the current user, the Moodle release, and every WS function this token
    /// may call. Call this first to verify the setup.
    #[tool(description = "Get Moodle site info: site name, current user, Moodle version, and the full list of web service functions enabled for this token. Use it first to verify connectivity and discover available functions.")]
    async fn site_info(
        &self,
        Parameters(_): Parameters<Empty>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.site_info_cached().await)
    }

    // ---------- courses ----------

    #[tool(description = "List the courses the user is enrolled in (name, dates, progress, summary).")]
    async fn list_courses(
        &self,
        Parameters(p): Parameters<ListCoursesParams>,
    ) -> Result<CallToolResult, McpError> {
        let user_id = p.user_id;
        finish(
            self.with_user_id(user_id, async |uid| {
                courses::list_courses(&self.client, uid).await
            })
            .await,
        )
    }

    #[tool(description = "Get the full content structure of a course: sections, modules/activities, and file resources with download URLs.")]
    async fn get_course_contents(
        &self,
        Parameters(p): Parameters<CourseContentsParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            courses::get_course_contents(
                &self.client,
                p.course_id,
                p.section_id,
                p.section_number,
                p.exclude_modules,
                p.exclude_contents,
            )
            .await,
        )
    }

    #[tool(description = "Search the course catalogue by keyword (courses beyond the user's enrolments).")]
    async fn search_courses(
        &self,
        Parameters(p): Parameters<SearchCoursesParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            courses::search_courses(&self.client, &p.search, p.limit.unwrap_or(20)).await,
        )
    }

    // ---------- assignments ----------

    #[tool(description = "List assignments (name, due date, description, submission rules) across enrolled courses or specific course ids, sorted by due date.")]
    async fn get_assignments(
        &self,
        Parameters(p): Parameters<AssignmentsParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            assignments::get_assignments(&self.client, p.course_ids.as_deref().unwrap_or(&[]))
                .await,
        )
    }

    #[tool(description = "Get the submission and grading status of an assignment: submission state (draft/submitted), feedback grade, teacher feedback files.")]
    async fn get_assignment_status(
        &self,
        Parameters(p): Parameters<AssignmentIdParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(assignments::get_assignment_status(&self.client, p.assignment_id).await)
    }

    #[tool(description = "Submit an online-text submission for an assignment. Note: if the assignment uses draft submissions, follow up with submit_assignment_for_grading.")]
    async fn submit_assignment_text(
        &self,
        Parameters(p): Parameters<SubmitTextParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            assignments::submit_assignment_text(
                &self.client,
                p.assignment_id,
                &p.text,
                p.format.as_deref().unwrap_or("html"),
            )
            .await,
        )
    }

    #[tool(description = "Submit uploaded draft file(s) (by draft item_id from upload_file) as the file submission for an assignment. Follow up with submit_assignment_for_grading if the assignment uses draft submissions.")]
    async fn submit_assignment_files(
        &self,
        Parameters(p): Parameters<SubmitFilesParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            assignments::submit_assignment_files(&self.client, p.assignment_id, p.item_id).await,
        )
    }

    #[tool(description = "Finalize the submission of an assignment for grading (required when the assignment has draft submissions enabled).")]
    async fn submit_assignment_for_grading(
        &self,
        Parameters(p): Parameters<SubmitForGradingParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            assignments::submit_for_grading(
                &self.client,
                p.assignment_id,
                p.accept_submission_statement.unwrap_or(true),
            )
            .await,
        )
    }

    // ---------- grades ----------

    #[tool(description = "Get detailed grade items for a user in a course: per-item grades, ranges, weights, teacher feedback.")]
    async fn get_grades(
        &self,
        Parameters(p): Parameters<GradesParams>,
    ) -> Result<CallToolResult, McpError> {
        let user_id = p.user_id;
        finish(
            self.with_user_id(user_id, async |uid| {
                grades::get_grades(&self.client, p.course_id, uid).await
            })
            .await,
        )
    }

    // ---------- calendar ----------

    #[tool(description = "Get calendar events (due dates, quizzes, meetings), optionally filtered by course ids and time window.")]
    async fn get_calendar_events(
        &self,
        Parameters(p): Parameters<CalendarParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            calendar::get_calendar_events(
                &self.client,
                p.course_ids.as_deref().unwrap_or(&[]),
                p.time_from,
                p.time_to,
                p.limit.unwrap_or(50),
            )
            .await,
        )
    }

    #[tool(description = "Facade: assignment deadlines coming up within N days across all enrolled courses, sorted by due date. The busy-student overview.")]
    async fn get_upcoming_deadlines(
        &self,
        Parameters(p): Parameters<DeadlinesParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(calendar::get_upcoming_deadlines(&self.client, p.days_ahead.unwrap_or(30)).await)
    }

    // ---------- forums ----------

    #[tool(description = "List forums in the given courses (or all enrolled courses).")]
    async fn get_forums(
        &self,
        Parameters(p): Parameters<ForumsParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(forums::get_forums(&self.client, p.course_ids.as_deref().unwrap_or(&[])).await)
    }

    #[tool(description = "List discussions in a forum (subject, author, replies, last activity).")]
    async fn get_forum_discussions(
        &self,
        Parameters(p): Parameters<DiscussionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let wsfunction = self
            .pick_function(&[
                "mod_forum_get_discussions",
                "mod_forum_get_forum_discussions",
                "mod_forum_get_forum_discussions_paginated",
            ])
            .await;
        finish(
            match wsfunction {
                Ok(f) => {
                    forums::get_forum_discussions(
                        &self.client,
                        &f,
                        p.forum_id,
                        p.page.unwrap_or(0),
                        p.per_page.unwrap_or(20),
                    )
                    .await
                }
                Err(e) => Err(e),
            },
        )
    }

    #[tool(description = "Read all posts in a discussion thread, in order (author, message, timestamps).")]
    async fn get_discussion_posts(
        &self,
        Parameters(p): Parameters<PostsParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            forums::get_discussion_posts(
                &self.client,
                p.discussion_id,
                p.page.unwrap_or(0),
                p.per_page.unwrap_or(30),
            )
            .await,
        )
    }

    #[tool(description = "Start a new discussion in a forum. WRITE operation.")]
    async fn create_forum_discussion(
        &self,
        Parameters(p): Parameters<NewDiscussionParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            forums::create_forum_discussion(
                &self.client,
                p.forum_id,
                &p.subject,
                &p.message,
                p.group_id,
            )
            .await,
        )
    }

    #[tool(description = "Reply to an existing forum post. WRITE operation.")]
    async fn reply_to_discussion(
        &self,
        Parameters(p): Parameters<ReplyParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            forums::reply_to_discussion(&self.client, p.post_id, &p.subject, &p.message).await,
        )
    }

    // ---------- messaging ----------

    #[tool(description = "Get the user's recent notifications (from teachers, system, etc.).")]
    async fn get_notifications(
        &self,
        Parameters(p): Parameters<NotificationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let wsfunction = self
            .pick_function(&["core_message_get_notifications", "core_message_get_messages"])
            .await;
        finish(
            match wsfunction {
                Ok(f) => {
                    self.with_user_id(None, async |uid| {
                        messaging::get_notifications(&self.client, &f, uid, p.read, p.limit.unwrap_or(20))
                            .await
                    })
                    .await
                }
                Err(e) => Err(e),
            },
        )
    }

    #[tool(description = "List the user's messaging conversations with unread counts and last messages.")]
    async fn get_conversations(
        &self,
        Parameters(p): Parameters<ConversationsParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.with_user_id(None, async |uid| {
                messaging::get_conversations(&self.client, uid, p.limit.unwrap_or(20)).await
            })
            .await,
        )
    }

    #[tool(description = "Send a direct message to a user. WRITE operation.")]
    async fn send_message(
        &self,
        Parameters(p): Parameters<SendMessageParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(messaging::send_message(&self.client, p.user_id, &p.text).await)
    }

    // ---------- users ----------

    #[tool(description = "Search users by keyword (requires user-listing permission on the site).")]
    async fn search_users(
        &self,
        Parameters(p): Parameters<SearchUsersParams>,
    ) -> Result<CallToolResult, McpError> {
        let wsfunction = self
            .pick_function(&["core_user_get_users", "core_message_message_search_users"])
            .await;
        finish(
            match wsfunction {
                Ok(f) => {
                    users::search_users(&self.client, &f, p.search.as_deref(), p.limit.unwrap_or(20))
                        .await
                }
                Err(e) => Err(e),
            },
        )
    }

    #[tool(description = "Look up a user by username, email, id, or idnumber (e.g. to find a teacher's user id before send_message).")]
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

    // ---------- quizzes ----------

    #[tool(description = "List quizzes in the given courses (or all enrolled courses) with open/close times and attempt limits.")]
    async fn get_quizzes(
        &self,
        Parameters(p): Parameters<QuizzesParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(quizzes::get_quizzes(&self.client, p.course_ids.as_deref().unwrap_or(&[])).await)
    }

    #[tool(description = "Get quiz attempts of a user: state, start/finish times, total marks.")]
    async fn get_quiz_attempts(
        &self,
        Parameters(p): Parameters<QuizAttemptsParams>,
    ) -> Result<CallToolResult, McpError> {
        let user_id = p.user_id;
        let status = match p.status {
            Some(QuizStatus::Finished) => "finished",
            Some(QuizStatus::Incomplete) => "incomplete",
            Some(QuizStatus::All) | None => "all",
        };
        finish(
            self.with_user_id(user_id, async |uid| {
                quizzes::get_quiz_attempts(&self.client, p.quiz_id, uid, status).await
            })
            .await,
        )
    }

    // ---------- files ----------

    #[tool(description = "Download a Moodle file (URL from get_course_contents or submission feedback). Small/decodable content is returned inline as text; other content as base64. The web service token is injected automatically.")]
    async fn download_file(
        &self,
        Parameters(p): Parameters<DownloadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        finish(files::download_file(&self.client, &p.url).await)
    }

    #[tool(description = "Upload a base64-encoded file to the user's draft file area; returns the draft item_id needed by submit_assignment_files.")]
    async fn upload_file(
        &self,
        Parameters(p): Parameters<UploadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let item_id = p.item_id.unwrap_or_else(crate::util::unique_item_id);
        finish(
            files::upload_draft_file(&self.client, &p.filename, &p.content_base64, item_id).await,
        )
    }

    // ---------- escape hatch ----------

    #[tool(description = "Call ANY Moodle web service function by name with free-form JSON parameters (advanced escape hatch; the function must be enabled for this token — see site_info). Nested arrays/objects are flattened to Moodle's PHP bracket notation automatically.")]
    async fn call_ws_function(
        &self,
        Parameters(p): Parameters<RawCallParams>,
    ) -> Result<CallToolResult, McpError> {
        let allowed = self.allowed_functions().await;
        let params = p.params.unwrap_or_else(|| serde_json::json!({}));
        finish(raw::call_ws_function(&self.client, &p.wsfunction, &params, &allowed).await)
    }
}

#[tool_handler(router = self.tool_router.clone())]
impl ServerHandler for MoodleMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("moodle-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Moodle MCP server (student-first). Recommended flow: site_info to verify the \
                 connection, list_courses to find course ids, then get_course_contents / \
                 get_assignments / get_upcoming_deadlines for coursework, get_grades for marks, \
                 forum and messaging tools for communication. Writing tools (submit_assignment_*, \
                 create_forum_discussion, reply_to_discussion, send_message) act as the token \
                 owner — always confirm intent before submitting or sending.",
            )
    }
}

/// Map a domain result onto an MCP tool result.
///
/// Moodle failures become *tool-level* errors (`isError: true`) so the model
/// can read the message and self-correct (wrong ids, missing permissions...).
/// Only serialization failures become protocol errors.
fn finish(result: Result<Value, MoodleError>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => {
            let value = strip_empty_warnings(value);
            let mut json = match serde_json::to_string(&value) {
                Ok(s) => s,
                Err(e) => {
                    return Err(McpError::internal_error(
                        format!("failed to serialize tool output: {e}"),
                        None,
                    ))
                }
            };
            if json.len() > MAX_OUTPUT_CHARS {
                json.truncate(MAX_OUTPUT_CHARS);
                json.push_str(" …[output truncated by moodle-mcp]");
            }
            Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
        }
        Err(e) => Ok(moodle_to_tool_result(e)),
    }
}
