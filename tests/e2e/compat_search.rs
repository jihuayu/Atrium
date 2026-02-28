use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn compat_search_supports_repo_label_and_text() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-search";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "hello world").await;

    let patch = app
        .as_admin()
        .patch(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .json(&serde_json::json!({"labels": ["bug"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 200);

    let search = app
        .as_anon()
        .get(&app.url(&format!(
            "/search/issues?q=hello%20repo:{}/{}%20label:bug",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(search.status(), 200);
    let payload: serde_json::Value = search.json().await.unwrap();
    assert!(payload["total_count"].as_i64().unwrap_or(0) >= 1);
}
