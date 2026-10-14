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
    // Did-you-mean: the 3.7-compatible variant is suggested.
    assert!(text.contains("Did you mean"), "{text}");
    assert!(text.contains("mod_forum_get_forum_discussions"));
}

// ---- Read-only / tool filtering (#10) ----

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
}

/// A tool hidden by read-only mode must reject calls, not just vanish from
/// the list.
#[tokio::test]
async fn read_only_mode_rejects_calls_to_hidden_tools() {
    let server = MockServer::start().await;
    site_info_mock().mount(&server).await;
    let mcp = spawn_server_with(&server, true, Vec::new()).await;

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
