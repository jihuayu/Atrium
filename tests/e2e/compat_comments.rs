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
