use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_admin_settings_permission_and_transfer() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-admin";

    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "seed").await;
    let _ = app.as_alice().get(&app.url("/user")).send().await.unwrap();

    let forbidden_get = app
        .as_bob()
        .get(&app.url(&format!("/api/v1/repos/{}/{}", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden_get.status(), 403);

    let admin_get = app
        .as_admin()
        .get(&app.url(&format!("/api/v1/repos/{}/{}", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_get.status(), 200);

    let invalid_patch = app
        .as_admin()
        .patch(&app.url(&format!("/api/v1/repos/{}/{}", owner, repo)))
        .json(&serde_json::json!({"admin_user_id": 999999}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_patch.status(), 422);

    let transfer = app
        .as_admin()
        .patch(&app.url(&format!("/api/v1/repos/{}/{}", owner, repo)))
        .json(&serde_json::json!({"admin_user_id": 2}))
        .send()
        .await
        .unwrap();
    assert_eq!(transfer.status(), 200);
    let transfer_body: serde_json::Value = transfer.json().await.unwrap();
    assert_eq!(transfer_body["admin_user_id"].as_i64(), Some(2));

    let alice_get = app
        .as_alice()
        .get(&app.url(&format!("/api/v1/repos/{}/{}", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_get.status(), 200);

    let old_admin_get = app
        .as_admin()
        .get(&app.url(&format!("/api/v1/repos/{}/{}", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(old_admin_get.status(), 403);
}
