use std::collections::HashSet;

use crate::common::{fixtures, TestApp};

#[tokio::test]
async fn native_threads_shape_and_cursor_pagination() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-threads";

    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "t1").await;
    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "t2").await;
    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "t3").await;

    let first = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads?limit=2&direction=asc&state=all",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);

    let body1: serde_json::Value = first.json().await.unwrap();
    let data1 = body1["data"].as_array().unwrap();
    assert_eq!(data1.len(), 2);
    assert!(data1[0].get("node_id").is_none());
    assert!(data1[0].get("locked").is_none());
    assert!(body1["pagination"]["has_more"].as_bool().unwrap_or(false));

    let next_cursor = body1["pagination"]["next_cursor"].as_str().unwrap();
    let second = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads?limit=2&direction=asc&state=all&cursor={}",
            owner, repo, next_cursor
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);

    let body2: serde_json::Value = second.json().await.unwrap();
    let data2 = body2["data"].as_array().unwrap();

    let mut ids = HashSet::new();
    for item in data1.iter().chain(data2.iter()) {
        ids.insert(item["id"].as_i64().unwrap());
    }
    assert_eq!(ids.len(), data1.len() + data2.len());
}

#[tokio::test]
async fn native_threads_auth_and_permission_branches() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-threads-perm";

    fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "admin seed").await;

    let unauth_create = app
        .as_anon()
        .post(&app.url(&format!("/api/v1/repos/{}/{}/threads", owner, repo)))
        .json(&serde_json::json!({"title": "unauth"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_create.status(), 401);

    let created = app
        .as_alice()
        .post(&app.url(&format!("/api/v1/repos/{}/{}/threads", owner, repo)))
        .json(&serde_json::json!({"title": "alice thread", "body": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created_body: serde_json::Value = created.json().await.unwrap();
    let number = created_body["number"].as_i64().unwrap();

    let bob_patch = app
        .as_bob()
        .patch(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}",
            owner, repo, number
        )))
        .json(&serde_json::json!({"title": "bob edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_patch.status(), 403);

    let alice_patch = app
        .as_alice()
        .patch(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}",
            owner, repo, number
        )))
        .json(&serde_json::json!({"title": "alice edit"}))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_patch.status(), 200);

    let alice_delete = app
        .as_alice()
        .delete(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}",
            owner, repo, number
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_delete.status(), 403);

    let admin_delete = app
        .as_admin()
        .delete(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads/{}",
            owner, repo, number
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_delete.status(), 204);
}

#[tokio::test]
async fn native_threads_get_desc_cursor_and_delete_not_found() {
    let app = TestApp::start().await;
    let owner = "e2e";
    let repo = "native-threads-desc";

    let n1 = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "t1").await;
    let _n2 = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "t2").await;
    let _n3 = fixtures::seed_issue(&app, &app.as_admin(), owner, repo, "t3").await;

    let first = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads?limit=2&direction=desc",
            owner, repo
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let first_body: serde_json::Value = first.json().await.unwrap();
    assert!(first_body["pagination"]["has_more"].as_bool().unwrap_or(false));
    let next_cursor = first_body["pagination"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_string();

    let second = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/repos/{}/{}/threads?limit=2&direction=desc&cursor={}",
            owner, repo, next_cursor
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);

    let get = app
        .as_anon()
        .get(&app.url(&format!("/api/v1/repos/{}/{}/threads/{}", owner, repo, n1)))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);

    let missing_delete = app
        .as_admin()
        .delete(&app.url(&format!("/api/v1/repos/{}/{}/threads/999999", owner, repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_delete.status(), 404);
}
