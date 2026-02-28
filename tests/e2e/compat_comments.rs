use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn compat_comment_pagination_and_count_updates() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-comments";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let cid = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "first").await;
    let _ = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "second").await;

    let list = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues/{}/comments?per_page=1&page=1",
            owner, repo, number
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    assert!(list.headers().get("link").is_some());

    let issue = app
        .as_anon()
        .get(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .send()
        .await
        .unwrap();
    let issue_json: serde_json::Value = issue.json().await.unwrap();
    assert_eq!(issue_json["comments"].as_i64(), Some(2));

    let del = app
        .as_alice()
        .delete(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}",
            owner, repo, cid
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);

    let issue2 = app
        .as_anon()
        .get(&app.url(&format!("/repos/{}/{}/issues/{}", owner, repo, number)))
        .send()
        .await
        .unwrap();
    let issue_json2: serde_json::Value = issue2.json().await.unwrap();
    assert_eq!(issue_json2["comments"].as_i64(), Some(1));
}

#[tokio::test]
async fn compat_comment_get_update_and_since_filter() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-comments-get-update";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let cid = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "first").await;
    let _ = fixtures::seed_comment(&app, &app.as_bob(), owner, repo, number, "second").await;

    let get = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}",
            owner, repo, cid
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);
    let get_body: serde_json::Value = get.json().await.unwrap();
    assert_eq!(get_body["id"].as_i64(), Some(cid));

    let bob_patch = app
        .as_bob()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}",
            owner, repo, cid
        )))
        .json(&serde_json::json!({"body": "bob edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_patch.status(), 403);

    let alice_patch = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}",
            owner, repo, cid
        )))
        .json(&serde_json::json!({"body": "alice edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_patch.status(), 200);
    let patch_body: serde_json::Value = alice_patch.json().await.unwrap();
    assert_eq!(patch_body["body"], "alice edit");

    let since_future = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues/{}/comments?since=2999-01-01T00:00:00Z",
            owner, repo, number
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(since_future.status(), 200);
    let since_payload: serde_json::Value = since_future.json().await.unwrap();
    assert_eq!(since_payload.as_array().map(|v| v.len()), Some(0));
}

#[tokio::test]
async fn compat_comment_validation_paths() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-comments-validation";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;

    let bad_create = app
        .as_alice()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/{}/comments",
            owner, repo, number
        )))
        .json(&serde_json::json!({"body": "   "}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_create.status(), 422);

    let created = app
        .as_alice()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/{}/comments",
            owner, repo, number
        )))
        .json(&serde_json::json!({"body": "ok"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created_body: serde_json::Value = created.json().await.unwrap();
    let cid = created_body["id"].as_i64().unwrap();

    let bad_patch = app
        .as_alice()
        .patch(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}",
            owner, repo, cid
        )))
        .json(&serde_json::json!({"body": ""}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_patch.status(), 422);
}
