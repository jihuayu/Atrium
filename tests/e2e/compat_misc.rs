use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn root_and_markdown_endpoints_work() {
    let app = TestApp::start().await;

    let root = app.as_anon().get(&app.url("/")).send().await.unwrap();
    assert_eq!(root.status(), 200);
    let root_text = root.text().await.unwrap();
    assert!(root_text.contains("Atrium"));

    let markdown = app
        .as_anon()
        .post(&app.url("/markdown"))
        .json(&serde_json::json!({"text": "**hello**"}))
        .send()
        .await
        .unwrap();
    assert_eq!(markdown.status(), 200);
    let html = markdown.text().await.unwrap();
    assert!(html.contains("<strong>hello</strong>"));
}

#[tokio::test]
async fn user_and_user_export_require_auth() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-user-export";

    let unauth_user = app.as_anon().get(&app.url("/user")).send().await.unwrap();
    assert_eq!(unauth_user.status(), 401);

    let auth_user = app.as_admin().get(&app.url("/user")).send().await.unwrap();
    assert_eq!(auth_user.status(), 200);
    let user_body: serde_json::Value = auth_user.json().await.unwrap();
    assert_eq!(user_body["login"], "admin");

    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "export me").await;

    let unauth_export = app
        .as_anon()
        .get(&app.url("/user/export"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_export.status(), 401);

    let auth_export = app
        .as_admin()
        .get(&app.url("/user/export"))
        .send()
        .await
        .unwrap();
    assert_eq!(auth_export.status(), 200);
    let export_body: serde_json::Value = auth_export.json().await.unwrap();
    assert_eq!(export_body["schema_version"].as_i64(), Some(1));
    assert!(export_body["repos"].as_array().unwrap().len() >= 1);
}
