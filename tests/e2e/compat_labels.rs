use crate::common::TestApp;

#[tokio::test]
async fn compat_labels_create_and_list() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-labels";

    let created = app
        .as_admin()
        .post(&app.url(&format!("/repos/{}/{}/labels", owner, repo)))
        .json(&serde_json::json!({
            "name": "bug",
            "color": "ff0000",
            "description": "Bug label"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);

    let listed = app
        .as_anon()
        .get(&app.url(&format!("/repos/{}/{}/labels", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), 200);

    let body: serde_json::Value = listed.json().await.unwrap();
    let labels = body.as_array().unwrap();
    assert!(labels.iter().any(|v| v["name"] == "bug"));
}

#[tokio::test]
async fn compat_labels_validate_name() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-labels-invalid";

    let invalid = app
        .as_admin()
        .post(&app.url(&format!("/repos/{}/{}/labels", owner, repo)))
        .json(&serde_json::json!({
            "name": "  ",
            "color": "ededed"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid.status(), 422);
}
