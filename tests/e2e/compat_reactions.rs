use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn compat_reaction_create_idempotent_and_delete_perm() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-reactions";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let comment_id = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "c1").await;

    // bob 创建 reaction → 201
    let first = app
        .as_bob()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "+1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);
    let reaction: serde_json::Value = first.json().await.unwrap();
    let reaction_id = reaction["id"].as_i64().unwrap();

    // bob 重复创建同一 reaction → 200（幂等）
    let second = app
        .as_bob()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "+1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);

    // alice 删除 bob 的 reaction → 403
    let forbidden = app
        .as_alice()
        .delete(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions/{}",
            owner, repo, comment_id, reaction_id
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), 403);

    // bob 删除自己的 reaction → 204
    let del_ok = app
        .as_bob()
        .delete(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions/{}",
            owner, repo, comment_id, reaction_id
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(del_ok.status(), 204);
}

#[tokio::test]
async fn compat_reaction_list_and_validation_paths() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "compat-reactions-list";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let comment_id = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "c1").await;

    let _ = app
        .as_alice()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "heart"}))
        .send()
        .await
        .unwrap();
    let _ = app
        .as_bob()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "+1"}))
        .send()
        .await
        .unwrap();

    let list = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions?per_page=1&page=1",
            owner, repo, comment_id
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    assert!(list.headers().get("link").is_some());
    let items: serde_json::Value = list.json().await.unwrap();
    assert_eq!(items.as_array().map(|v| v.len()), Some(1));

    let bad_content = app
        .as_alice()
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "invalid"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_content.status(), 422);

    let missing_comment = app
        .as_anon()
        .get(&app.url(&format!(
            "/repos/{}/{}/issues/comments/{}/reactions",
            owner, repo, 999999
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_comment.status(), 404);
}
