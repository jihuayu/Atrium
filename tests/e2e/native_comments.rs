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
