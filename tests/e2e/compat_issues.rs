use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn compat_create_requires_auth_and_list_is_public() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-issues-public";

    let anon_create = app
        .as_anon()
        .post(&app.url(&format!("/repos/{}/{}/issues", owner, repo)))
        .json(&serde_json::json!({"title": "unauthorized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(anon_create.status(), 401);

    let _number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "first issue").await;

    let list = app
        .as_anon()
        .get(&app.url(&format!("/repos/{}/{}/issues", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let payload: serde_json::Value = list.json().await.unwrap();
    assert!(payload.is_array());
    assert!(!payload.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn compat_patch_forbidden_for_non_author() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-issues-perm";

    let number = fixtures::seed_issue(&app, &app.as_alice(), owner, repo, "alice issue").await;

    // bob 不是作者，不能 patch → 403
    let patch = app
        .as_bob()
        .patch(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .json(&serde_json::json!({"title": "bob edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 403);

    // alice 是作者，可以 patch → 200
    let patch_ok = app
        .as_alice()
        .patch(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .json(&serde_json::json!({"title": "alice edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_ok.status(), 200);
    let body: serde_json::Value = patch_ok.json().await.unwrap();
    assert_eq!(body["title"], "alice edit");
}

#[tokio::test]
async fn compat_issue_close_and_soft_delete_behaviors() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-issues-close-delete";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "close me").await;

    let close = app
        .as_admin()
        .patch(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .json(&serde_json::json!({"state": "closed"}))
        .send()
        .await
        .unwrap();
    assert_eq!(close.status(), 200);
    let close_body: serde_json::Value = close.json().await.unwrap();
    assert_eq!(close_body["state"], "closed");
    assert!(close_body["closed_at"].as_str().is_some());

    let del = app
        .as_admin()
        .delete(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}",
            owner, repo, number
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);

    let get_after_delete = app
        .as_anon()
        .get(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .send()
        .await
        .unwrap();
    assert_eq!(get_after_delete.status(), 404);
}
