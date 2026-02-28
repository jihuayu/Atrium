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

#[tokio::test]
async fn compat_issue_filters_and_validation_paths() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-issues-filters";

    let alice_number =
        fixtures::seed_issue(&app, &app.as_alice(), owner, repo, "alice issue").await;
    let _bob_number = fixtures::seed_issue(&app, &app.as_bob(), owner, repo, "bob issue").await;

    let bad_create = app
        .as_alice()
        .post(&app.url(&format!("/repos/{}/{}/issues", owner, repo)))
        .json(&serde_json::json!({"title": "   "}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_create.status(), 422);

    let with_labels = app
        .as_alice()
        .post(&app.url(&format!("/repos/{}/{}/issues", owner, repo)))
        .json(&serde_json::json!({"title": "with labels", "labels": ["enhancement"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(with_labels.status(), 201);

    let close_and_label = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/{}",
            owner, repo, alice_number
        )))
        .json(&serde_json::json!({
            "state": "closed",
            "state_reason": "completed",
            "labels": ["bug"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(close_and_label.status(), 200);

    let by_creator = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues?state=all&creator=alice",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(by_creator.status(), 200);
    let by_creator_payload: serde_json::Value = by_creator.json().await.unwrap();
    assert!(by_creator_payload
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["number"].as_i64() == Some(alice_number)));

    let by_label = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues?state=all&labels=bug",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(by_label.status(), 200);
    let by_label_payload: serde_json::Value = by_label.json().await.unwrap();
    assert!(by_label_payload
        .as_array()
        .unwrap()
        .iter()
        .any(|v| { v["number"].as_i64() == Some(alice_number) }));

    let since_future = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues?since=2999-01-01T00:00:00Z",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(since_future.status(), 200);
    let since_payload: serde_json::Value = since_future.json().await.unwrap();
    assert_eq!(since_payload.as_array().map(|v| v.len()), Some(0));

    let invalid_state = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/{}",
            owner, repo, alice_number
        )))
        .json(&serde_json::json!({"state": "invalid-state"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_state.status(), 422);

    let empty_title = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/{}",
            owner, repo, alice_number
        )))
        .json(&serde_json::json!({"title": "   "}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_title.status(), 422);

    let update_body = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/{}",
            owner, repo, alice_number
        )))
        .json(&serde_json::json!({"body": "updated body"}))
        .send()
        .await
        .unwrap();
    assert_eq!(update_body.status(), 200);

    let reopen = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/{}",
            owner, repo, alice_number
        )))
        .json(&serde_json::json!({"state": "open"}))
        .send()
        .await
        .unwrap();
    assert_eq!(reopen.status(), 200);
    let reopen_body: serde_json::Value = reopen.json().await.unwrap();
    assert!(reopen_body["closed_at"].is_null());
}
