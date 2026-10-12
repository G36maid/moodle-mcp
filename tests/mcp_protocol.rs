//! MCP protocol smoke test: serve MoodleMcp over an in-memory duplex
//! transport, act as an MCP client, list tools and call them.

use moodle_mcp::client::MoodleClient;
use moodle_mcp::config::Config;
use moodle_mcp::tools::MoodleMcp;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn spawn_server(
    server: &MockServer,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let client = MoodleClient::new(Config {
        base_url: server.uri(),
        token: "testtoken".into(),
        timeout_secs: 10,
    })
    .expect("client");

    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);

    tokio::spawn(async move {
        let svc = MoodleMcp::new(Arc::new(client));
        match svc.serve(server_transport).await {
            Ok(running) => {
                // Keep the RunningService alive for the whole session.
                let _ = running.waiting().await;
            }
            Err(e) => eprintln!("SERVER ERROR: {e:?}"),
        }
    });

    ().serve(client_transport).await.expect("client connect")
}

#[tokio::test]
async fn lists_all_tools_over_mcp() {
    let server = MockServer::start().await;
    let mcp = spawn_server(&server).await;

    let tools = mcp.peer().list_tools(Default::default()).await.expect("list_tools");
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.len() >= 25,
        "expected >=25 tools, got {}: {names:?}",
        names.len()
    );
    for expected in [
        "site_info",
        "list_courses",
        "get_course_contents",
        "get_assignments",
        "get_assignment_status",
        "submit_assignment_text",
        "upload_file",
        "submit_assignment_files",
        "submit_assignment_for_grading",
        "get_grades",
        "get_calendar_events",
        "get_upcoming_deadlines",
        "get_forums",
        "get_forum_discussions",
        "get_discussion_posts",
        "create_forum_discussion",
        "reply_to_discussion",
        "get_notifications",
        "get_conversations",
        "send_message",
        "search_users",
        "get_user",
        "get_quizzes",
        "get_quiz_attempts",
        "download_file",
        "call_ws_function",
    ] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}

#[tokio::test]
async fn calls_site_info_over_mcp() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sitename": "Test Moodle",
            "userid": 7,
            "username": "student1",
            "release": "5.1 (Build: 20251006)",
            "functions": ["core_webservice_get_site_info"]
        })))
        .mount(&server)
        .await;

    let mcp = spawn_server(&server).await;
    let result = mcp
        .peer()
        .call_tool(CallToolRequestParams::new("site_info"))
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
        .call_tool(CallToolRequestParams::new("get_upcoming_deadlines"))
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
async fn raw_call_rejects_unknown_function() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sitename": "T", "userid": 1,
            "functions": ["core_webservice_get_site_info", "core_enrol_get_users_courses"]
        })))
        .mount(&server)
        .await;

    let mcp = spawn_server(&server).await;
    let result = mcp
        .peer()
        .call_tool(
            CallToolRequestParams::new("call_ws_function").with_arguments(
                json!({
                    "wsfunction": "core_message_send_instant_messages",
                    "params": {"messages": []}
                })
                .as_object()
                .expect("object")
                .clone(),
            ),
        )
        .await
        .expect("call_tool");

    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");
    assert!(text.contains("not enabled for this token"));
}
