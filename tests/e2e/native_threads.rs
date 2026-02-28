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
