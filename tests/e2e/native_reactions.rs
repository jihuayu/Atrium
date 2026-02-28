use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_reactions_add_and_delete_by_content() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-reactions";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let comment_id =
        fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "comment").await;

    let unauth_add = app
        .as_anon()
        .post(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "heart"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_add.status(), 401);

    let add = app
        .as_bob()
        .post(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "heart"}))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), 201);

    let add_again = app
        .as_bob()
        .post(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}/reactions",
            owner, repo, comment_id
        )))
        .json(&serde_json::json!({"content": "heart"}))
        .send()
        .await
        .unwrap();
    assert_eq!(add_again.status(), 200);

    let alice_delete = app
        .as_alice()
        .delete(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}/reactions/heart",
            owner, repo, comment_id
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_delete.status(), 204);

    let still_exists = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}",
            owner, repo, comment_id
        )))
        .send()
        .await
        .unwrap();
    let still_body: serde_json::Value = still_exists.json().await.unwrap();
    assert_eq!(still_body["reactions"]["heart"].as_i64(), Some(1));

    let del = app
        .as_bob()
        .delete(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}/reactions/heart",
            owner, repo, comment_id
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);

    let after_delete = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/comments/{}",
            owner, repo, comment_id
        )))
        .send()
        .await
        .unwrap();
    let after_body: serde_json::Value = after_delete.json().await.unwrap();
    assert_eq!(after_body["reactions"]["heart"].as_i64(), Some(0));
}
