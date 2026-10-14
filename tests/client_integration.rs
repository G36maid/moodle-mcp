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
        timezone: None,
        read_only: false,
        disabled_tools: Vec::new(),
    })
    .expect("client")
}

/// Any unmatched request on the mock server fails the test loudly — these
/// tests assert both what IS sent and what is NOT.
fn expect_no_other_calls() -> Mock {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("UNEXPECTED CALL"))
}

/// On 3.7 `core_user_get_users` is enabled but rejects the `search`
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
        .and(body_string_contains("userid=5"))
        .and(body_string_contains("search=chen"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "contacts": [],
            "noncontacts": [{"id": 9, "fullname": "Wang Chen"}]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let out =
        moodle_mcp::tools::users::search_users(&client, "core_user_get_users", 5, Some("chen"), 20)
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

/// HTML resources render as markdown, not raw tags.
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

/// On 3.7 the grades-overview API omits `coursename` — empty names
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

/// A missing save_to directory must fail with an error that names the
/// directory and suggests the fix — never silently create directory
/// structure on the agent's behalf (the filesystem-server convention).
#[tokio::test]
async fn download_missing_directory_names_it_and_suggests_the_fix() {
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
    let missing_dir = std::env::temp_dir().join(format!(
        "moodle-mcp-absent-{}/nested/deep",
        std::process::id()
    ));
    let save_to = missing_dir.join("notes.txt");
    let url = format!(
        "{}/webservice/pluginfile.php/12/course/section/notes.txt",
        server.uri()
    );
    let err = moodle_mcp::tools::files::download_file(&client, &url, save_to.to_str().unwrap())
        .await
        .expect_err("missing directory must fail");
    assert!(err.to_string().contains("does not exist"), "{err:?}");
    assert!(
        err.to_string().contains("create the directory first"),
        "{err:?}"
    );
    assert!(!save_to.exists(), "no file should be written");
    assert!(!missing_dir.exists(), "no directory should be created");
}

// ---- Regression tests: issues found on a live 3.7 site ----

#[tokio::test]
async fn exception_without_errorcode_is_still_detected() {
    // mod_forum_get_discussion_posts can return
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

// ---- Version quirks and bad paths ----

#[tokio::test]
async fn schedule_sends_course_ids_and_time_window() {
    // get_schedule must scope the query to the requested courses only;
    // 3.7 needs the nested params events[courseids] + options[timestart/timeend].
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
    // On 3.7, gradereport_user_get_grade_items without userid
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
    // A never-submitted assignment (submission AND feedback both null)
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
/// draft area. That must surface as an error, not `uploaded: true`.
#[tokio::test]
async fn upload_reports_per_file_error_records() {
    let server = MockServer::start().await;
    // upload.php takes one POST per file; discriminate the scenarios by the
    // multipart filename.
    Mock::given(method("POST"))
        .and(path("/webservice/upload.php"))
        .and(body_string_contains("dup.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "filename": "dup.txt",
            "filepath": "/tmp/php7ikf5k",
            "size": 9,
            "errortype": "filenameexist",
            "error": "The file dup.txt already exists"
        }])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/upload.php"))
        .and(body_string_contains("big.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "filename": "big.bin",
            "errortype": "fileoversized",
            "error": "The file is too large"
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
        Some("dup.txt"),
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

    // fileoversized: the other per-file errortype upload.php reports.
    let err = moodle_mcp::tools::files::upload_file_from_path(
        &client,
        tmp.to_str().unwrap(),
        Some("big.bin"),
        424243,
    )
    .await
    .expect_err("oversize record must fail the upload");
    match &err {
        MoodleError::Api { errorcode, .. } => assert_eq!(errorcode, "fileoversized"),
        other => panic!("expected Api error, got {other:?}"),
    }
    assert!(err.to_tool_message().contains("upload size limit"));
    std::fs::remove_file(&tmp).ok();
}

/// Successful upload records pass through unchanged.
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

// ---- File downloads: rewrite + non-file response detection ----

/// Plain /pluginfile.php URLs (what the browser shows) must be rewritten to
/// the token-aware /webservice/pluginfile.php before fetching.
#[tokio::test]
async fn download_rewrites_plain_pluginfile_to_webservice() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/webservice/pluginfile.php/26/mod_assign/introattachment/0/HW1.pdf",
        ))
        .and(wiremock::matchers::query_param("token", "testtoken"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("%PDF-1.4 fake")
                .append_header("content-type", "application/pdf"),
        )
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let plain = format!(
        "{}/pluginfile.php/26/mod_assign/introattachment/0/HW1.pdf",
        server.uri()
    );
    let resp = client
        .download_file(&plain)
        .await
        .expect("rewritten download");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.url().path(),
        "/webservice/pluginfile.php/26/mod_assign/introattachment/0/HW1.pdf"
    );
}

/// A redirect that ends on the login page must be an error, not a saved
/// HTML "file" — for both fetch_file and download_file.
#[tokio::test]
async fn login_page_redirect_is_an_error_not_a_file() {
    let server = MockServer::start().await;
    let login = format!("{}/login/index.php", server.uri());
    Mock::given(method("GET"))
        .and(path("/webservice/pluginfile.php/1/bad/n.txt"))
        .respond_with(ResponseTemplate::new(303).append_header("location", login.as_str()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/login/index.php"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body>please log in</body></html>")
                .append_header("content-type", "text/html"),
        )
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let url = format!("{}/webservice/pluginfile.php/1/bad/n.txt", server.uri());
    let err = moodle_mcp::tools::files::fetch_file(&client, &url)
        .await
        .expect_err("login redirect must fail fetch");
    assert!(err.to_string().contains("login page"), "{err:?}");

    let save_to =
        std::env::temp_dir().join(format!("moodle-mcp-login-{}.html", std::process::id()));
    let err = moodle_mcp::tools::files::download_file(&client, &url, save_to.to_str().unwrap())
        .await
        .expect_err("login redirect must fail download");
    assert!(err.to_string().contains("login page"), "{err:?}");
    assert!(!save_to.exists(), "no file should be written");
}

/// webservice/pluginfile.php replies HTTP 200 + JSON when the token is
/// invalid — that payload must become an error, while a legitimate JSON
/// attachment (no errorcode key) still comes through as a file.
#[tokio::test]
async fn json_responses_split_moodle_errors_from_real_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/webservice/pluginfile.php/1/notes.txt"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({
                    "error": "Invalid token - token not found",
                    "errorcode": "invalidtoken",
                    "stacktrace": null,
                    "debuginfo": null,
                    "reproductionlink": null
                }))
                .append_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;
    let legit = r#"{"grades": [75, 100, 65]}"#;
    Mock::given(method("GET"))
        .and(path("/webservice/pluginfile.php/1/data.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(legit)
                .append_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let bad = format!("{}/webservice/pluginfile.php/1/notes.txt", server.uri());
    let good = format!("{}/webservice/pluginfile.php/1/data.json", server.uri());

    // Error payload → tool error on both paths, nothing written.
    let fetch_err = moodle_mcp::tools::files::fetch_file(&client, &bad)
        .await
        .expect_err("JSON error must fail fetch");
    let save_bad =
        std::env::temp_dir().join(format!("moodle-mcp-jsonerr-{}.txt", std::process::id()));
    let dl_err = moodle_mcp::tools::files::download_file(&client, &bad, save_bad.to_str().unwrap())
        .await
        .expect_err("JSON error must fail download");
    for e in [&fetch_err, &dl_err] {
        assert!(e.to_string().contains("invalidtoken"), "{e:?}");
    }
    assert!(!save_bad.exists(), "no file should be written");

    // Real JSON file → inline content / bytes on disk, flagged as neither.
    let out = moodle_mcp::tools::files::fetch_file(&client, &good)
        .await
        .expect("legitimate JSON is a file, not an error");
    assert_eq!(out["content"], json!(legit));

    let save = std::env::temp_dir().join(format!("moodle-mcp-jsonok-{}.json", std::process::id()));
    let out = moodle_mcp::tools::files::download_file(&client, &good, save.to_str().unwrap())
        .await
        .expect("legitimate JSON downloads");
    assert_eq!(out["size_bytes"], json!(legit.len()));
    assert_eq!(std::fs::read_to_string(&save).unwrap(), legit);
    std::fs::remove_file(&save).ok();
}

/// mod_forum_get_discussion_posts attachments carry the link in `url`
/// (stored_file_exporter, 3.7+); `fileurl` alone is not reliable there.
#[tokio::test]
async fn discussion_post_attachments_read_url_field() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_forum_get_discussion_posts",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "posts": [{
                "id": 101,
                "subject": "Lecture notes",
                "message": "<p>see attachment</p>",
                "author": {"id": 5, "fullname": "Prof Lin"},
                "timecreated": 1760851200,
                "attachments": [{
                    "filename": "week3.pdf",
                    "filenameshort": "week3.pdf",
                    "filesize": 1234,
                    "url": "/pluginfile.php/26/mod_forum/attachment/101/week3.pdf"
                }]
            }],
            "warnings": []
        })))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::forums::get_discussion_posts(&client, 42)
        .await
        .expect("posts");
    let att = &out["posts"][0]["attachments"][0];
    assert_eq!(att["filename"], json!("week3.pdf"));
    assert_eq!(
        att["fileurl"],
        json!("/pluginfile.php/26/mod_forum/attachment/101/week3.pdf")
    );
}

// Gradebook-graded assignments keep feedback.grade.grade at the -1 sentinel;
// the real grade is in gradefordisplay. Plugin HTML (online text, feedback
// comment) renders as markdown.
#[tokio::test]
async fn assignment_grade_shows_display_not_sentinel() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_assignments",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "courses": [{
                "id": 3, "fullname": "Physics",
                "assignments": [{
                    "id": 7, "course": 3, "cmid": 71, "name": "Lab report",
                    "allowsubmissionsfromdate": 0, "duedate": 0, "cutoffdate": 0,
                    "submissiondraft": 0, "intro": "<p>write it up</p>",
                    "introattachments": []
                }]
            }],
            "warnings": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_submission_status",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assignments": [{
                "gradingstatus": "graded",
                "submissions": [{
                    "status": "submitted", "attemptnumber": 0,
                    "plugins": [{
                        "type": "onlinetext",
                        "editorfields": [{
                            "name": "onlinetext",
                            "text": "<p>our <strong>lab notes</strong><br /></p>"
                        }]
                    }]
                }],
                "feedback": {
                    "grade": {"grade": "-1.00000", "gradeddate": 1760851200},
                    "gradefordisplay": "75.00&nbsp;/&nbsp;100.00",
                    "plugins": [{
                        "type": "comments",
                        "editorfields": [{
                            "name": "comment",
                            "text": "<p><span>111/115<br />良好</span><br /></p>"
                        }]
                    }]
                },
                "previousattempts": []
            }],
            "warnings": []
        })))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let (md, structured) = moodle_mcp::tools::assignments::get_assignment(&client, 7)
        .await
        .expect("assignment");
    assert_eq!(structured["graded"], json!(true));
    assert_eq!(
        structured["grade"],
        Value::Null,
        "sentinel must not surface"
    );
    assert_eq!(structured["grade_display"], json!("75.00 / 100.00"));
    // #12: plugin HTML becomes markdown in both channels — tags gone,
    // structure kept (line breaks, bold).
    assert_eq!(
        structured["submission"]["onlinetext"],
        json!("our **lab notes**")
    );
    let fb = structured["feedback_comment"].as_str().unwrap_or_default();
    assert!(
        fb.contains("111/115") && fb.contains("良好") && !fb.contains('<'),
        "feedback_comment: {fb}"
    );
    assert!(md.contains("**Grade**: 75.00 / 100.00"), "{md}");
    assert!(md.contains("**lab notes**"), "{md}");
    assert!(md.contains("111/115"), "{md}");
    assert!(!md.contains("<span>"), "{md}");
    assert!(!md.contains("-1.00000"), "{md}");
    assert!(!md.contains("\"75.00"), "no JSON quotes in markdown: {md}");
}

/// Moodle 4.0+: omitting `read` sends a single read=2 ("both") call, so the
/// unread half is not dropped; the read flag is derived from timeread.
#[tokio::test]
async fn notifications_omitted_read_sends_both_on_modern_sites() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wsfunction=core_message_get_messages"))
        .and(body_string_contains("read=2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "messages": [
                {"id": 1, "subject": "unread one", "timecreated": 200, "timeread": null,
                 "useridfrom": 3, "userfromfullname": "Prof Lin", "smallmessage": "hi"},
                {"id": 2, "subject": "read one", "timecreated": 100, "timeread": 150,
                 "useridfrom": 3, "userfromfullname": "Prof Lin", "smallmessage": "yo"}
            ],
            "warnings": []
        })))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::messaging::get_notifications(
        &client,
        "core_message_get_messages",
        5,
        None,
        20,
        true,
    )
    .await
    .expect("notifications");
    assert_eq!(out["count"], json!(2));
    assert_eq!(
        out["notifications"][0]["read"],
        json!(false),
        "timeread null = unread"
    );
    assert_eq!(
        out["notifications"][1]["read"],
        json!(true),
        "timeread set = read"
    );
}

/// Up to 3.11 (or an unreadable release) `read` is a boolean with no "both"
/// value: omitting it must merge read=0 + read=1, deduped by id.
#[tokio::test]
async fn notifications_omitted_read_merges_two_calls_on_legacy_sites() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wsfunction=core_message_get_messages"))
        .and(body_string_contains("read=0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "messages": [
                {"id": 1, "subject": "unread one", "timecreated": 200, "timeread": null,
                 "useridfrom": 3, "userfromfullname": "Prof Lin"}
            ],
            "warnings": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wsfunction=core_message_get_messages"))
        .and(body_string_contains("read=1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "messages": [
                {"id": 2, "subject": "read one", "timecreated": 150, "timeread": 160,
                 "useridfrom": 3, "userfromfullname": "Prof Lin"},
                {"id": 1, "subject": "unread one (dup)", "timecreated": 200, "timeread": 180,
                 "useridfrom": 3, "userfromfullname": "Prof Lin"}
            ],
            "warnings": []
        })))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::messaging::get_notifications(
        &client,
        "core_message_get_messages",
        5,
        None,
        20,
        false,
    )
    .await
    .expect("notifications");
    assert_eq!(out["count"], json!(2), "duplicate id must be deduped");
    assert_eq!(out["notifications"][0]["id"], json!(1), "newest first");
    assert_eq!(
        out["notifications"][0]["read"],
        json!(true),
        "flag from merged read half"
    );
    assert_eq!(out["notifications"][1]["id"], json!(2));
    assert_eq!(out["notifications"][1]["read"], json!(true));
}

/// The primary search function (core_user_get_users) sends the 3.8+ `search`
/// criteria key and parses the {users} response — the fallback test only
/// covers the message_search_users path.
#[tokio::test]
async fn search_users_primary_sends_criteria_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("wsfunction=core_user_get_users"))
        .and(body_string_contains("criteria%5B0%5D%5Bkey%5D=search"))
        .and(body_string_contains("criteria%5B0%5D%5Bvalue%5D=chen"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "users": [{"id": 9, "fullname": "Wang Chen", "username": "chen", "email": "c@x.edu"}]
        })))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let out =
        moodle_mcp::tools::users::search_users(&client, "core_user_get_users", 5, Some("chen"), 20)
            .await
            .expect("primary search");
    assert_eq!(out["count"], json!(1));
    assert_eq!(out["users"][0]["username"], json!("chen"));
}

/// message_popup_get_popup_notifications accepts neither `read` nor
/// `newestfirst`; the popup branch must send neither and pass the item's
/// own `read` flag through.
#[tokio::test]
async fn popup_notifications_send_no_read_filter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(
            "wsfunction=message_popup_get_popup_notifications",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "notifications": [
                {"id": 5, "subject": "grade released", "read": true,
                 "timecreated": 100, "smallmessage": "hi"},
                {"id": 6, "subject": "new post", "read": false,
                 "timecreated": 200, "smallmessage": "yo"}
            ],
            "warnings": []
        })))
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::messaging::get_notifications(
        &client,
        "message_popup_get_popup_notifications",
        5,
        None,
        20,
        false,
    )
    .await
    .expect("popup notifications");

    let reqs = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&reqs[0].body);
    assert!(
        !body.contains("read="),
        "popup branch must not send read: {body}"
    );
    assert!(
        !body.contains("newestfirst"),
        "popup branch must not send newestfirst: {body}"
    );

    assert_eq!(out["count"], json!(2));
    assert_eq!(
        out["notifications"][0]["read"],
        json!(true),
        "popup read flag passes through"
    );
    assert_eq!(out["notifications"][1]["read"], json!(false));
}

/// Non-convertible binaries must fail with a pointer at download_file
/// instead of stuffing bytes into the conversation.
#[tokio::test]
async fn fetch_rejects_non_convertible_binary() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/webservice/pluginfile.php/1/asset.zip"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes([0x50u8, 0x4b, 0xff, 0xfe, 0x00, 0x01, 0x02])
                .append_header("content-type", "application/zip"),
        )
        .mount(&server)
        .await;
    expect_no_other_calls().mount(&server).await;

    let client = client_for(&server).await;
    let err = moodle_mcp::tools::files::fetch_file(
        &client,
        &format!("{}/webservice/pluginfile.php/1/asset.zip", server.uri()),
    )
    .await
    .expect_err("binary must be rejected");
    assert!(err.to_string().contains("download_file"), "{err:?}");
}

/// Empty or omitted search keywords are rejected client-side with zero HTTP
/// traffic — an empty query makes Moodle match every user on the site and
/// the request times out (observed live on NTNU 3.7: exactly 60 s, then a
/// misleading permission error).
#[tokio::test]
async fn search_users_rejects_blank_queries_without_http() {
    let server = MockServer::start().await;
    expect_no_other_calls().mount(&server).await;
    let client = client_for(&server).await;
    for bad in [None, Some(""), Some("   \t ")] {
        let err = moodle_mcp::tools::users::search_users(
            &client,
            "core_message_message_search_users",
            5,
            bad,
            20,
        )
        .await
        .expect_err("blank query must be rejected");
        assert!(
            err.to_string().contains("provide search keywords"),
            "{err:?}"
        );
    }
}

/// A blank save_to fails with a plain-language message instead of the odd
/// `failed to create : No such file or directory`.
#[tokio::test]
async fn download_rejects_blank_save_to() {
    let server = MockServer::start().await;
    expect_no_other_calls().mount(&server).await;
    let client = client_for(&server).await;
    let err = moodle_mcp::tools::files::download_file(
        &client,
        "https://moodle3.example.com/webservice/pluginfile.php/1/f.pdf",
        "   ",
    )
    .await
    .expect_err("blank save_to must be rejected");
    assert!(err.to_string().contains("save_to is empty"), "{err:?}");
}
