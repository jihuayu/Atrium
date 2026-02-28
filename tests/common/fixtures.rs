use serde_json::Value;

use super::{AuthClient, TestApp};

pub async fn seed_issue(
    app: &TestApp,
    client: &AuthClient,
    owner: &str,
    repo: &str,
    title: &str,
) -> i64 {
    let response = client
        .post(&app.url(&format!("/repos/{}/{}/issues", owner, repo)))
        .json(&serde_json::json!({"title": title, "body": "seed body"}))
        .send()
        .await
        .unwrap();
    if response.status() != 201 {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        panic!("seed_issue failed: status={}, body={}", status, body);
    }

    let payload: Value = response.json().await.unwrap();
    payload["number"].as_i64().unwrap()
}

pub async fn seed_comment(
    app: &TestApp,
    client: &AuthClient,
    owner: &str,
    repo: &str,
    number: i64,
    body: &str,
) -> i64 {
    let response = client
        .post(&app.url(&format!(
            "/repos/{}/{}/issues/{}/comments",
            owner, repo, number
        )))
        .json(&serde_json::json!({"body": body}))
        .send()
        .await
        .unwrap();
    if response.status() != 201 {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        panic!("seed_comment failed: status={}, body={}", status, payload);
    }

    let payload: Value = response.json().await.unwrap();
    payload["id"].as_i64().unwrap()
}
