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
