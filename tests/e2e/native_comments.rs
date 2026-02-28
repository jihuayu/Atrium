use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_comments_order_desc_works() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-comments";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;
    let c1 = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "a").await;
    let c2 = fixtures::seed_comment(&app, &app.as_alice(), owner, repo, number, "b").await;
    assert!(c2 > c1);

    let response = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}/comments?order=desc&limit=10",
            owner, repo, number
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    let items = body["data"].as_array().unwrap();
    assert!(items.len() >= 2);
    let first_id = items[0]["id"].as_i64().unwrap();
    let second_id = items[1]["id"].as_i64().unwrap();
    assert!(first_id > second_id);
}

#[tokio::test]
async fn native_comments_auth_and_permission_branches() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-comments-perm";

    let number = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "thread").await;

    let unauth_create = app
        .as_anon()
        .post(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}/comments",
            owner, repo, number
        )))
        .json(&serde_json::json!({"body": "unauth"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_create.status(), 401);

    let created = app
        .as_alice()
        .post(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}/comments",
            owner, repo, number
        )))
        .json(&serde_json::json!({"body": "alice comment"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created_body: serde_json::Value = created.json().await.unwrap();
    let id = created_body["id"].as_i64().unwrap();

    let bob_patch = app
        .as_bob()
        .patch(&app.url(&format!("/api/v1/repos/{}/{}/comments/{}", owner, repo, id)))
        .json(&serde_json::json!({"body": "bob edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_patch.status(), 403);

    let alice_patch = app
        .as_alice()
        .patch(&app.url(&format!("/api/v1/repos/{}/{}/comments/{}", owner, repo, id)))
        .json(&serde_json::json!({"body": "alice edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_patch.status(), 200);

    let bob_delete = app
        .as_bob()
        .delete(&app.url(&format!("/api/v1/repos/{}/{}/comments/{}", owner, repo, id)))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_delete.status(), 403);

    let alice_delete = app
        .as_alice()
        .delete(&app.url(&format!("/api/v1/repos/{}/{}/comments/{}", owner, repo, id)))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_delete.status(), 204);
}
