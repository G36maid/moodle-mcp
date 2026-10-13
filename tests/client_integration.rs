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
        .call::<Value>("anything", &json!({}))
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
        .call::<Value>("anything", &json!({}))
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

// ---- 回歸測試:真實 NTNU 3.7 發現的問題 ----

#[tokio::test]
async fn exception_without_errorcode_is_still_detected() {
    // NTNU 實測:mod_forum_get_discussion_posts 對某些討論串回
    // {exception: "Error", message: "..."} —— 沒有 errorcode 欄位。
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
        .call::<Value>(
            "mod_forum_get_discussion_posts",
            &json!({"discussionid": 1}),
        )
        .await
        .expect_err("must fail");
    assert!(matches!(err, MoodleError::Api { .. }), "got: {err}");
}

// ---- 回歸測試:pi e2e 發現的問題 ----

#[tokio::test]
async fn schedule_sends_course_ids_and_time_window() {
    // pi e2e 發現:get_schedule 曾忽略 course_ids(混入他課事件)。
    // 3.7 的巢狀參數必須是 events[courseids] + options[timestart/timeend]。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("events%5Bcourseids%5D%5B0%5D=59944"))
        .and(body_string_contains("options%5Btimestart%5D="))
        .and(body_string_contains("options%5Btimeend%5D="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [
                {"id": 1, "name": "HW due", "timestart": 1790000000,
                 "eventtype": "due", "modulename": "assign", "courseid": 59944}
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let (out, hints) = moodle_mcp::tools::schedule::get_schedule(&client, &[59944], None, None)
        .await
        .expect("schedule");
    assert_eq!(out["count"], json!(1));
    assert_eq!(out["schedule"][0]["course_id"], json!(59944));
    assert!(hints.is_empty());
}

#[tokio::test]
async fn grades_always_send_own_userid_3_7_regression() {
    // pi e2e 發現:3.7 的 gradereport_user_get_grade_items 不帶 userid 會走
    // 「所有用戶」路徑 → required_capability_exception。必須永遠帶 userid。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("userid=44785"))
        .and(body_string_contains("courseid=61716"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usergrades": [{
                "courseid": 61716,
                "gradeitems": [
                    {"itemname": "Lab01", "gradeformatted": "100.00",
                     "grademaxformatted": "100.00"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let out = moodle_mcp::tools::grades::get_grades(&client, Some(61716), 44785)
        .await
        .expect("grades");
    assert_eq!(out["items"][0]["grade"], json!("100.00"));
}

#[tokio::test]
async fn null_submission_renders_as_not_submitted() {
    // pi e2e 發現:從未提交的作業(submission 與 feedback 皆 null)曾誤報
    // "empty submission status" —— 合法狀態,應渲染為 not submitted。
    let server = MockServer::start().await;
    // 廣泛的 mock:任何 mod_assign 呼叫都回對應形狀
    Mock::given(method("POST"))
        .and(body_string_contains(
            "wsfunction=mod_assign_get_assignments",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "courses": [{"id": 61716, "fullname": "課程", "assignments": [
                {"id": 144948, "cmid": 1, "name": "HW", "submissiondraft": 0}
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
    let (md, structured) = moodle_mcp::tools::assignments::get_assignment(&client, 144948)
        .await
        .expect("get_assignment");
    assert!(md.contains("**Submission**: no submission yet"), "{md}");
    assert_eq!(
        structured["submission"]["status"],
        json!("no submission yet")
    );
}
