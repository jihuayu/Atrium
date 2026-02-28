use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_reactions_add_and_delete_by_content() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-reactions";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let comment_id =
        fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "comment").await;

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
}
