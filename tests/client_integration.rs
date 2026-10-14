//! End-to-end tests: MoodleClient against a mocked Moodle REST endpoint.

use moodle_mcp::client::MoodleClient;
use moodle_mcp::config::Config;
use moodle_mcp::error::MoodleError;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn client_for(server: &MockServer) -> MoodleClient {
    MoodleClient::new(Config {
        base_url: server.uri(),
        token: "testtoken".into(),
        timeout_secs: 10,
    })
    .expect("client")
}

/// Any unmatched request on the mock server fails the test loudly — these
/// tests assert both what IS sent and what is NOT.
fn expect_no_other_calls() -> Mock {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("UNEXPECTED CALL"))
}

/// 3.7 regression: `core_user_get_users` is enabled but rejects the `search`
/// criteria key (added in 3.8) with `invalidparameter` — the tool must
/// transparently retry with `core_message_message_search_users`.
#[tokio::test]
async fn search_users_falls_back_on_invalidparameter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("wsfunction=core_user_get_users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exception": "invalid_parameter_exception",
            "errorcode": "invalidparameter",
            "message": "Missing required key in single structure: search"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "wsfunction=core_message_message_search_users",
        ))
        .and(body_string_contains("searchtext=chen"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "users": [{"id": 9, "fullname": "Wang Chen", "username": "chen"}]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let out =
        moodle_mcp::tools::users::search_users(&client, "core_user_get_users", Some("chen"), 20)
            .await
            .expect("fallback search");
    assert_eq!(out["count"], json!(1));
    assert_eq!(out["users"][0]["fullname"], json!("Wang Chen"));
}

/// An empty `text` would overwrite the online text with blank content —
/// refused client-side, with no HTTP call made at all.
#[tokio::test]
async fn submit_assignment_rejects_blank_text() {
    let server = MockServer::start().await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let err =
        moodle_mcp::tools::assignments::submit_assignment(&client, 1002, Some("   "), None, false)
            .await
            .expect_err("must refuse");
    let msg = err.to_tool_message();
    assert!(msg.contains("refusing to overwrite"), "{msg}");
    assert_eq!(server.received_requests().await.unwrap().len(), 0);
}

/// HTML resources render as markdown, not raw tags (live finding: an .html
/// course resource used to come back as a raw `<div>…` blob).
#[tokio::test]
async fn fetch_converts_html_to_markdown() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/page.html"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(
                    "<h1>Chap2</h1><table><tr><th>Q</th></tr><tr><td>7</td></tr></table>",
                )
                .insert_header("content-type", "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::files::fetch_file(&client, &format!("{}/page.html", server.uri()))
        .await
        .expect("fetch");
    assert_eq!(out["format"], json!("html"));
    let md = out["markdown"].as_str().unwrap();
    assert!(md.contains("Chap2"), "heading lost: {md}");
    assert!(md.contains("|"), "table lost: {md}");
    assert!(!md.contains("<table>"), "raw tags leaked: {md}");
}

/// 3.7 regression: the grades-overview API omits `coursename` — empty names
/// are filled from the enrolled-course roster.
#[tokio::test]
async fn overview_course_names_are_enriched_from_roster() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("wsfunction=gradereport_overview"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "grades": [
                {"courseid": 101, "coursename": "", "grade": "92.00"},
                {"courseid": 102, "coursename": "", "grade": "88.00"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "wsfunction=core_enrol_get_users_courses",
        ))
        .and(body_string_contains("userid=42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 101, "fullname": "Course One"},
            {"id": 102, "fullname": "Course Two"}
        ])))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::grades::get_grades(&client, None, 42)
        .await
        .expect("overview");
    let out = moodle_mcp::tools::grades::enrich_overview_course_names(&client, out, 42)
        .await
        .expect("enrich");
    assert_eq!(out["courses"][0]["course"], json!("Course One"));
    assert_eq!(out["courses"][1]["course"], json!("Course Two"));
}

#[tokio::test]
async fn call_parses_success_payload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wstoken=testtoken"))
        .and(body_string_contains(
            "wsfunction=core_enrol_get_users_courses",
        ))
        .and(body_string_contains("moodlewsrestformat=json"))
        .and(body_string_contains("userid=42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "fullname": "Math 101", "shortname": "M101"}
        ])))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let v: Value = client
        .call("core_enrol_get_users_courses", &json!({"userid": 42}))
        .await
        .expect("call");
    assert_eq!(v[0]["fullname"], "Math 101");
}

#[tokio::test]
async fn call_flattens_php_arrays() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("courseids%5B0%5D=2"))
        .and(body_string_contains("criteria%5B0%5D%5Bkey%5D=search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let v: Value = client
        .call(
            "some_function",
            &json!({"courseids": [2], "criteria": [{"key": "search"}]}),
        )
        .await
        .expect("call");
    assert_eq!(v["ok"], true);
}

#[tokio::test]
async fn call_maps_exception_payload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exception": "moodle_exception",
            "errorcode": "invalidtoken",
            "message": "Invalid token - token not found"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let err: MoodleError = client
        .call("anything", &json!({}))
        .await
        .expect_err("must fail");
    match err {
        MoodleError::Api {
            ref errorcode,
            ref message,
            ..
        } => {
            assert_eq!(errorcode, "invalidtoken");
            assert!(message.contains("Invalid token"));
        }
        other => panic!("expected Api error, got {other}"),
    }
    // Tool message should carry actionable hints for the LLM.
    let msg = err.to_tool_message();
    assert!(msg.contains("MOODLE_TOKEN"));
}

#[tokio::test]
async fn non_json_body_is_reported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>login page</html>"))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let err = client
        .call("anything", &json!({}))
        .await
        .expect_err("must fail");
    let msg = err.to_tool_message();
    assert!(msg.to_lowercase().contains("web services"));
}

#[tokio::test]
async fn download_injects_token_into_pluginfile_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/webservice/pluginfile.php/12/course/section/notes.txt",
        ))
        .and(wiremock::matchers::query_param("token", "testtoken"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("hello file")
                .append_header("content-type", "text/plain"),
        )
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let url = format!(
        "{}/webservice/pluginfile.php/12/course/section/notes.txt",
        server.uri()
    );
    let resp = client.download_file(&url).await.expect("download");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.bytes().await.expect("bytes"), "hello file");
}

/// download_file must create missing parent directories for save_to —
/// agents construct nested temp paths without mkdir-ing first (#1).
#[tokio::test]
async fn download_creates_missing_parent_directories() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/webservice/pluginfile.php/12/course/section/notes.txt",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("hello file")
                .append_header("content-type", "text/plain"),
        )
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let save_to = std::env::temp_dir().join(format!(
        "moodle-mcp-test-{}/nested/deep/notes.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(save_to.parent().unwrap());
    let url = format!(
        "{}/webservice/pluginfile.php/12/course/section/notes.txt",
        server.uri()
    );
    let out = moodle_mcp::tools::files::download_file(&client, &url, save_to.to_str().unwrap())
        .await
        .expect("download with missing parents");
    assert_eq!(out["saved_to"], json!(save_to.to_str().unwrap()));
    assert_eq!(out["size_bytes"], json!(10));
    assert_eq!(std::fs::read_to_string(&save_to).unwrap(), "hello file");
    std::fs::remove_dir_all(save_to.parent().unwrap()).unwrap();
}

// ---- Regression tests: issues found on a live 3.7 site ----

#[tokio::test]
async fn exception_without_errorcode_is_still_detected() {
    // Live 3.7 finding: mod_forum_get_discussion_posts returns
    // {exception: "Error", message: "…"} for some discussions — with NO
    // errorcode field at all.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exception": "Error",
            "message": "Call to a member function get_forum_id() on null"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let err = client
        .call(
            "mod_forum_get_discussion_posts",
            &json!({"discussionid": 1}),
        )
        .await
        .expect_err("must fail");
    assert!(matches!(err, MoodleError::Api { .. }), "got: {err}");
}

// ---- Regression tests: issues found by agent e2e testing ----

#[tokio::test]
async fn schedule_sends_course_ids_and_time_window() {
    // e2e finding: get_schedule used to ignore course_ids (events from other
    // courses leaked in). 3.7 needs the nested params events[courseids] +
    // options[timestart/timeend].
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("events%5Bcourseids%5D%5B0%5D=102"))
        .and(body_string_contains("options%5Btimestart%5D="))
        .and(body_string_contains("options%5Btimeend%5D="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [
                {"id": 1, "name": "HW due", "timestart": 1790000000,
                 "eventtype": "due", "modulename": "assign", "courseid": 102}
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let (out, hints) = moodle_mcp::tools::schedule::get_schedule(&client, &[102], None, None)
        .await
        .expect("schedule");
    assert_eq!(out["count"], json!(1));
    assert_eq!(out["schedule"][0]["course_id"], json!(102));
    assert!(hints.is_empty());
}

#[tokio::test]
async fn grades_always_send_own_userid_3_7_regression() {
    // e2e finding: on 3.7, gradereport_user_get_grade_items without userid
    // takes the all-users path → required_capability_exception. The caller's
    // own userid must ALWAYS be sent.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("userid=42"))
        .and(body_string_contains("courseid=101"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usergrades": [{
                "courseid": 101,
                "gradeitems": [
                    {"itemname": "Lab01", "gradeformatted": "100.00",
                     "grademaxformatted": "100.00"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::grades::get_grades(&client, Some(101), 42)
        .await
        .expect("grades");
    assert_eq!(out["items"][0]["grade"], json!("100.00"));
}

#[tokio::test]
async fn null_submission_renders_as_not_submitted() {
    // e2e finding: a never-submitted assignment (submission AND feedback
    // both null) used to be misreported as "empty submission status" — it
    // is a legitimate state and must render as not submitted.
    let server = MockServer::start().await;
    // Broad mocks: any mod_assign call returns the matching shape.
    Mock::given(method("POST"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_assignments",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "courses": [{"id": 101, "fullname": "Demo Course", "assignments": [
                {"id": 1001, "cmid": 1, "name": "HW", "submissiondraft": 0}
            ]}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_submission_status",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "lastattempt": {
                "submission": null,
                "gradingstatus": "nougraded",
                "submissionsenabled": true
            },
            "feedback": null
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let (md, structured) = moodle_mcp::tools::assignments::get_assignment(&client, 1001)
        .await
        .expect("get_assignment");
    assert!(md.contains("**Submission**: no submission yet"), "{md}");
    assert_eq!(
        structured["submission"]["status"],
        json!("no submission yet")
    );
}

/// upload.php replies HTTP 200 with a per-file error record (no exception
/// key) when it cannot store the file — e.g. the name already exists in the
/// draft area. That must surface as an error, not `uploaded: true` (#9).
#[tokio::test]
async fn upload_reports_per_file_error_records() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/upload.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "filename": "dup.txt",
            "filepath": "/tmp/php7ikf5k",
            "size": 9,
            "errortype": "filenameexist",
            "error": "The file dup.txt already exists"
        }])))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let tmp = std::env::temp_dir().join(format!("moodle-mcp-upl-{}.txt", std::process::id()));
    std::fs::write(&tmp, b"dup data").unwrap();
    let client = client_for(&server).await;
    let err = moodle_mcp::tools::files::upload_file_from_path(
        &client,
        tmp.to_str().unwrap(),
        None,
        424242,
    )
    .await
    .expect_err("error record must fail the upload");
    match &err {
        MoodleError::Api { errorcode, .. } => assert_eq!(errorcode, "filenameexist"),
        other => panic!("expected Api error, got {other:?}"),
    }
    let msg = err.to_tool_message();
    assert!(msg.contains("already exists"), "{msg}");
    assert!(msg.contains("different filename"), "{msg}");
    std::fs::remove_file(&tmp).ok();
}

/// Successful upload records keep flowing through unchanged (#9 guard).
#[tokio::test]
async fn upload_success_records_still_report_item_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/upload.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "itemid": 77,
            "filename": "notes.txt",
            "filepath": "/",
            "filesize": 8
        }])))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let tmp = std::env::temp_dir().join(format!("moodle-mcp-uplok-{}.txt", std::process::id()));
    std::fs::write(&tmp, b"12345678").unwrap();
    let client = client_for(&server).await;
    let out =
        moodle_mcp::tools::files::upload_file_from_path(&client, tmp.to_str().unwrap(), None, 77)
            .await
            .expect("successful upload");
    assert_eq!(out["uploaded"], json!(true));
    assert_eq!(out["item_id"], json!(77));
    assert_eq!(out["filesize"], json!(8));
    std::fs::remove_file(&tmp).ok();
}
