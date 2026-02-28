use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_export_requires_admin() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-export-perm";

    let _ = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;

    // alice 非 admin → 403
    let forbidden = app
        .as_alice()
        .get(&app.url(&format!("/api/v1/repos/{}/{}/export", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), 403);

    // admin → 200
    let ok = app
        .as_admin()
        .get(&app.url(&format!("/api/v1/repos/{}/{}/export", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body: serde_json::Value = ok.json().await.unwrap();
    assert!(body["threads"].as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn native_export_json_and_csv() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-export-data";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let _ =
        fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "hello export").await;

    let json_resp = app
        .as_admin()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/export?format=json",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(json_resp.status(), 200);
    let json_body: serde_json::Value = json_resp.json().await.unwrap();
    assert_eq!(json_body["repo"]["owner"], owner);
    assert_eq!(json_body["repo"]["name"], repo);
    assert!(json_body["threads"].as_array().unwrap().len() >= 1);

    let csv_resp = app
        .as_admin()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/export?format=csv",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(csv_resp.status(), 200);
    let ct = csv_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/csv"));
    let csv_text = csv_resp.text().await.unwrap();
    assert!(csv_text.contains("thread_number"));
}

#[tokio::test]
async fn native_export_since_filters_old_records() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-export-since";

    let _ = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "old thread").await;

    let future = "2999-01-01T00:00:00Z";
    let resp = app
        .as_admin()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/export?format=json&since={}",
            owner, repo, future
        )))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["threads"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn native_export_validates_query_params() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-export-params";

    let _ = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;

    let bad_format = app
        .as_admin()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/export?format=xml",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_format.status(), 400);

    let bad_since = app
        .as_admin()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/export?since=not-a-time",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_since.status(), 400);
}
