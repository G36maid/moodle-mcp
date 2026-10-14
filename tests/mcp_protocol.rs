//! MCP protocol smoke test: serve MoodleMcp over an in-memory duplex
//! transport, act as an MCP client, list tools and call them.

use moodle_mcp::client::MoodleClient;
use moodle_mcp::config::Config;
use moodle_mcp::tools::MoodleMcp;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn spawn_server(server: &MockServer) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    spawn_server_with(server, false, Vec::new()).await
}

async fn spawn_server_with(
    server: &MockServer,
    read_only: bool,
    disabled_tools: Vec<String>,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let client = MoodleClient::new(Config {
        base_url: server.uri(),
        token: "testtoken".into(),
        timeout_secs: 10,
        timezone: None,
        read_only: false,
        disabled_tools: Vec::new(),
    })
    .expect("client");

    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);

    tokio::spawn(async move {
        let svc = MoodleMcp::new_with_policy(Arc::new(client), read_only, disabled_tools);
        match svc.serve(server_transport).await {
            // Keep the RunningService alive for the whole session.
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(e) => eprintln!("SERVER ERROR: {e:?}"),
        }
    });

    ().serve(client_transport).await.expect("client connect")
}

fn site_info_mock() -> Mock {
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sitename": "Test Moodle",
            "userid": 7,
            "username": "student1",
            "release": "5.1 (Build: 20251006)",
            "functions": ["core_webservice_get_site_info", "core_enrol_get_users_courses"]
        })))
}

#[tokio::test]
async fn lists_all_tools_over_mcp() {
    let server = MockServer::start().await;
    site_info_mock().mount(&server).await;
    let mcp = spawn_server(&server).await;

    let tools = mcp
        .peer()
        .list_tools(Default::default())
        .await
        .expect("list_tools");
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    // Pinned exactly: 22 tools is the design contract — additions or losses
    // are breaking changes that must be conscious.
    assert_eq!(
        names.len(),
        22,
        "expected exactly 22 tools, got {}: {names:?}",
        names.len()
    );
    for expected in [
        "whoami",
        "list_courses",
        "get_course_overview",
        "get_forum_discussions",
        "get_discussion_posts",
        "create_forum_discussion",
        "reply_to_discussion",
        "get_assignment",
        "submit_assignment",
        "fetch_file",
        "download_file",
        "upload_file",
        "get_grades",
        "get_schedule",
        "get_notifications",
        "get_conversations",
        "send_message",
        "get_quiz_attempts",
        "get_user",
        "search_users",
        "call_ws_function",
        "list_functions",
    ] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}

#[tokio::test]
async fn calls_whoami_over_mcp() {
    let server = MockServer::start().await;
    site_info_mock().mount(&server).await;

    let mcp = spawn_server(&server).await;
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("whoami"))
        .await
        .expect("call_tool");

    assert!(!result.is_error.unwrap_or(false));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");
    assert!(text.contains("Test Moodle"));
    assert!(text.contains("student1"));
    let sc = result.structured_content.as_ref().expect("structured");
    assert_eq!(sc["timezone"], json!("UTC"));
    assert_eq!(sc["read_only"], json!(false));
    assert_eq!(sc["mcp_version"], json!(env!("CARGO_PKG_VERSION")));
}

#[tokio::test]
async fn moodle_failure_becomes_tool_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exception": "webservice_access_exception",
            "errorcode": "accesscontrolexception",
            "message": "Access control exception"
        })))
        .mount(&server)
        .await;

    let mcp = spawn_server(&server).await;
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("list_courses"))
        .await
        .expect("call_tool returns normally");

    // Tool-level error (isError: true) with a readable message — NOT a
    // protocol error.
    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");
    assert!(text.contains("accesscontrolexception"));
}

#[tokio::test]
async fn raw_call_rejects_string_params_loudly() {
    // LLM clients sometimes serialize `params` as a JSON-encoded string. The
    // call must fail loudly — silently flattening it would invoke the
    // function with NO parameters.
    let server = MockServer::start().await;
    site_info_mock().mount(&server).await;

    let mcp = spawn_server(&server).await;
    let args = json!({
        "wsfunction": "core_webservice_get_site_info",
        // A string that is NOT a JSON object — must be rejected loudly.
        // (A string CONTAINING an object is accepted and parsed.)
        "params": "[1, 2, 3]"
    })
    .as_object()
    .expect("object")
    .clone();
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("call_ws_function").with_arguments(args))
        .await
        .expect("call_tool");

    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");
    assert!(text.contains("params must be a JSON object"), "{text}");
}

#[tokio::test]
async fn raw_call_rejects_unknown_function_with_suggestion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sitename": "T", "userid": 1,
            "functions": [
                {"name": "core_webservice_get_site_info"},
                {"name": "mod_forum_get_forum_discussions"}
            ]
        })))
        .mount(&server)
        .await;

    let mcp = spawn_server(&server).await;
    let args = json!({
        "wsfunction": "mod_forum_get_discussions",
        "params": {"forumid": 1}
    })
    .as_object()
    .expect("object")
    .clone();
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("call_ws_function").with_arguments(args))
        .await
        .expect("call_tool");

    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");
    // "this configuration" — the function may exist in Moodle but be absent
    // from the service/site setup the token belongs to.
    assert!(
        text.contains("not enabled for this configuration"),
        "{text}"
    );
    // Did-you-mean: the 3.7-compatible variant is suggested.
    assert!(text.contains("Did you mean"), "{text}");
    assert!(text.contains("mod_forum_get_forum_discussions"));
}

// ---- Read-only / tool filtering ----

/// MOODLE_READ_ONLY must hide the 4 write tools + upload_file +
/// call_ws_function (the escape hatch could otherwise invoke write
/// functions directly) — 22 - 6 = 16 tools remain.
#[tokio::test]
async fn read_only_mode_hides_write_upload_and_escape_tools() {
    let server = MockServer::start().await;
    site_info_mock().mount(&server).await;
    let mcp = spawn_server_with(&server, true, Vec::new()).await;

    let tools = mcp
        .peer()
        .list_tools(Default::default())
        .await
        .expect("list_tools");
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), 16, "expected 16 tools, got {names:?}");
    for hidden in [
        "create_forum_discussion",
        "reply_to_discussion",
        "submit_assignment",
        "send_message",
        "upload_file",
        "call_ws_function",
    ] {
        assert!(
            !names.contains(&hidden),
            "{hidden} must be hidden in read-only mode"
        );
    }
    // list_functions stays — exploring what the token may call is harmless.
    assert!(names.contains(&"list_functions"));
    assert!(names.contains(&"get_assignment"));

    // Calls to hidden tools are rejected by the router, not just unlisted.
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("submit_assignment"))
        .await;
    assert!(
        result.is_err(),
        "call to a disabled tool must be a protocol error"
    );
}

/// MOODLE_DISABLE_TOOLS drops individual tools on top of the full surface.
#[tokio::test]
async fn disable_tools_filters_individual_tools() {
    let server = MockServer::start().await;
    site_info_mock().mount(&server).await;
    let mcp = spawn_server_with(
        &server,
        false,
        vec!["send_message".into(), "get_grades".into()],
    )
    .await;

    let tools = mcp
        .peer()
        .list_tools(Default::default())
        .await
        .expect("list_tools");
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), 20, "expected 20 tools, got {names:?}");
    assert!(!names.contains(&"send_message"));
    assert!(!names.contains(&"get_grades"));
    assert!(
        names.contains(&"submit_assignment"),
        "non-disabled write tools remain"
    );
}

/// limit=0 on the limiting tools means the default (20), not "one item" —
/// the normalization lives in the tool layer, so this goes through the real
/// MCP server.
#[tokio::test]
async fn limit_zero_means_default_twenty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=core_webservice_get_site_info",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sitename": "Test Moodle",
            "userid": 7,
            "release": "4.5.4 (Build: 20250417)",
            "functions": ["core_webservice_get_site_info", "core_message_get_messages"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wsfunction=core_message_get_messages"))
        .and(body_string_contains("limitnum=20"))
        .and(body_string_contains("read=2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "messages": [
                {"id": 1, "subject": "a", "timecreated": 300, "timeread": null},
                {"id": 2, "subject": "b", "timecreated": 200, "timeread": null},
                {"id": 3, "subject": "c", "timecreated": 100, "timeread": 50}
            ],
            "warnings": []
        })))
        .mount(&server)
        .await;

    let mcp = spawn_server(&server).await;
    let args = json!({"limit": 0}).as_object().expect("object").clone();
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("get_notifications").with_arguments(args))
        .await
        .expect("call_tool");
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    // count is asserted via the mock having matched limitnum=20; shape check:
    assert!(!result.content.is_empty());
}

#[tokio::test]
async fn course_overview_renders_section_summaries_and_descriptions_in_full() {
    // Regression for #13: the overview used to drop section summaries and
    // activity descriptions (300-char per-field cap, never rendered in the
    // markdown doc) — most of what a student sees on the course page.
    let server = MockServer::start().await;
    // wiremock is first-mounted-first-served: the catch-all site_info mock
    // must come after every function-specific mock below.
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wsfunction=core_course_get_contents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 1,
                "name": "Week 1: Intro",
                "summary": "<p><a href=\"https://example.com/syllabus.pdf\">Syllabus</a></p>\
                            <h3>Prerequisites</h3><ul><li>Python</li><li>C++</li></ul>"
            },
            {
                "id": 2,
                "name": "Week 2: Protocols",
                "summary": "",
                "modules": [
                    {
                        "id": 10,
                        "modname": "assign",
                        "name": "Lab Assignment 01",
                        "description": format!(
                            "<p><b>Experiment</b></p><ul><li>{}</li><li>TAIL-MARKER-AFTER-300</li></ul>",
                            "step ".repeat(80)
                        ),
                        "contents": [
                            {"type": "file", "filename": "lab.pdf", "filesize": 123,
                             "mimetype": "application/pdf",
                             "fileurl": "https://example.com/pluginfile.php/lab.pdf"}
                        ]
                    }
                ]
            }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_assignments",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "courses": [
                {"id": 1, "fullname": "Test Course", "assignments": [
                    {"id": 5, "cmid": 10, "name": "Lab Assignment 01",
                     "duedate": 1_800_000_000, "submissiondraft": 1, "introattachments": []}
                ]}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_quiz_get_quizzes_by_courses",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"courses": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_forum_get_forums_by_courses",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 3, "name": "Announcements", "type": "news"}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=gradereport_user_get_grade_items",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usergrades": [
                {"courseid": 1, "coursename": "Test Course",
                 "gradeitems": [{"itemname": "Lab 01", "gradeformatted": "90.00"}]}
            ]
        })))
        .mount(&server)
        .await;
    site_info_mock().mount(&server).await;

    let mcp = spawn_server(&server).await;
    let result = mcp
        .peer()
        .call_tool(
            CallToolRequestParams::new("get_course_overview")
                .with_arguments(json!({"course_id": 1}).as_object().expect("object").clone()),
        )
        .await
        .expect("call_tool");
    assert!(!result.is_error.unwrap_or(false), "{result:?}");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");

    // Summary rendered in the doc with its link and demoted heading.
    assert!(
        text.contains("[Syllabus](https://example.com/syllabus.pdf)"),
        "{text}"
    );
    assert!(text.contains("##### Prerequisites"), "{text}");
    // not hijacking the outline: no line starts with a higher-level heading
    assert!(!text.contains("\n### Prerequisites"), "{text}");
    // Description rendered past where the old 300-char cap used to cut.
    assert!(text.contains("TAIL-MARKER-AFTER-300"), "{text}");
    // File listing still intact under the module bullet.
    assert!(text.contains("- file: lab.pdf (123 bytes)"), "{text}");

    // Structured JSON carries the full text too (never truncated).
    let sc = result.structured_content.as_ref().expect("structured");
    let desc = sc["sections"][1]["modules"][0]["description"]
        .as_str()
        .unwrap();
    assert!(desc.contains("TAIL-MARKER-AFTER-300"), "{desc}");
    assert_eq!(sc["grades"]["items"], json!([["Lab 01", "90.00"]]));
}

#[tokio::test]
async fn course_overview_marks_restricted_and_hidden_items() {
    // Restricted activities must not read as freely usable: the course page
    // greys them out with availabilityinfo text (real NTNU case — an exam
    // sign-up "available from 2027-01-11").
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wsfunction=core_course_get_contents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 1,
                "name": "Week 1",
                "summary": "",
                "availabilityinfo": "Available from <strong>1 October 2026</strong>"
            },
            {
                "id": 2,
                "name": "Week 2",
                "summary": "",
                "modules": [
                    {
                        "id": 10, "modname": "assign", "name": "Exam Sign-up",
                        "description": "",
                        "availabilityinfo": "自<strong>2027年01月11日,21:00</strong>起即可用",
                        "uservisible": false, "visible": 1
                    },
                    {"id": 11, "modname": "forum", "name": "Announcements"}
                ]
            }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_assignments",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "courses": [{"id": 1, "fullname": "Test Course", "assignments": []}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_quiz_get_quizzes_by_courses",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"courses": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_forum_get_forums_by_courses",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=gradereport_user_get_grade_items",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"usergrades": []})))
        .mount(&server)
        .await;
    site_info_mock().mount(&server).await;

    let mcp = spawn_server(&server).await;
    let result = mcp
        .peer()
        .call_tool(
            CallToolRequestParams::new("get_course_overview")
                .with_arguments(json!({"course_id": 1}).as_object().expect("object").clone()),
        )
        .await
        .expect("call_tool");
    assert!(!result.is_error.unwrap_or(false), "{result:?}");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");

    // Section-level restriction line under the heading.
    assert!(
        text.contains("Restricted: Available from **1 October 2026**"),
        "{text}"
    );
    // Module line carries the restriction and the unavailable marker.
    assert!(
        text.contains(
            "- [assign] Exam Sign-up (module_id: 10) — restricted: 自**2027年01月11日,21:00**起即可用; unavailable"
        ),
        "{text}"
    );
    // Normal items stay quiet — no marks, no availability fields.
    assert!(
        text.contains("- [forum] Announcements (module_id: 11)\n"),
        "{text}"
    );

    let sc = result.structured_content.as_ref().expect("structured");
    assert_eq!(
        sc["sections"][0]["availability_info"],
        json!("Available from **1 October 2026**")
    );
    let exam = &sc["sections"][1]["modules"][0];
    assert_eq!(exam["uservisible"], json!(false));
    assert!(exam.get("visible").is_none(), "visible=1 must not appear");
    let normal = &sc["sections"][1]["modules"][1];
    assert!(normal.get("availability_info").is_none(), "{normal}");
    assert!(normal.get("uservisible").is_none(), "{normal}");
}
