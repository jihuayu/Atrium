use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_labels_admin_crud_and_permissions() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-labels";

    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "seed").await;

    let unauth_create = app
        .as_anon()
        .post(&app.url(&format!("/api/v1/repos/{}/{}/labels", owner, repo)))
        .json(&serde_json::json!({"name": "bug", "color": "d73a4a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_create.status(), 401);

    let non_admin_create = app
        .as_alice()
        .post(&app.url(&format!("/api/v1/repos/{}/{}/labels", owner, repo)))
        .json(&serde_json::json!({"name": "bug", "color": "d73a4a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(non_admin_create.status(), 403);

    let admin_create = app
        .as_admin()
        .post(&app.url(&format!("/api/v1/repos/{}/{}/labels", owner, repo)))
        .json(&serde_json::json!({"name": "bug", "color": "d73a4a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_create.status(), 201);

    let list = app
        .as_anon()
        .get(&app.url(&format!("/api/v1/repos/{}/{}/labels", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let labels: serde_json::Value = list.json().await.unwrap();
    assert!(labels
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["name"] == "bug"));

    let non_admin_delete = app
        .as_bob()
        .delete(&app.url(&format!("/api/v1/repos/{}/{}/labels/bug", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(non_admin_delete.status(), 403);

    let admin_delete = app
        .as_admin()
        .delete(&app.url(&format!("/api/v1/repos/{}/{}/labels/bug", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_delete.status(), 204);
}
