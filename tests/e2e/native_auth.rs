use crate::common::TestApp;

#[tokio::test]
async fn native_auth_me_requires_token_or_bypass() {
    let app = TestApp::start().await;

    // 无 token → 401
    let unauth = app
        .as_anon()
        .get(&app.url("/api/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    // 正确 bypass → 200
    let ok = app
        .as_admin()
        .get(&app.url("/api/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(body["login"], "admin");
}

#[tokio::test]
async fn native_auth_google_returns_501_when_not_configured() {
    let app = TestApp::start().await;

    // google 未配置 → 501
    let resp = app
        .as_anon()
        .post(&app.url("/api/v1/auth/google"))
        .json(&serde_json::json!({"token": "fake"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 501);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "not_configured");

    // apple 未配置 → 同样 501
    let resp2 = app
        .as_anon()
        .post(&app.url("/api/v1/auth/apple"))
        .json(&serde_json::json!({"token": "fake"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 501);
}

#[tokio::test]
async fn native_auth_wrong_bypass_secret_falls_back_to_401() {
    let app = TestApp::start().await;

    // 错误 bypass secret → 401
    let resp = reqwest::Client::new()
        .get(app.url("/api/v1/auth/me"))
        .header("Authorization", "testuser wrong:9:hacker:hacker@test.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // 正确 bypass secret → 200
    let ok = app
        .as_admin()
        .get(&app.url("/api/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(body["login"], "admin");
}

#[tokio::test]
async fn native_auth_refresh_and_session_delete_require_valid_token() {
    let app = TestApp::start().await;

    let refresh_no_token = app
        .as_anon()
        .post(&app.url("/api/v1/auth/refresh"))
        .send()
        .await
        .unwrap();
    assert_eq!(refresh_no_token.status(), 401);

    let refresh_bad_token = app
        .as_anon()
        .post(&app.url("/api/v1/auth/refresh"))
        .json(&serde_json::json!({"refresh_token": "invalid.token.value"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refresh_bad_token.status(), 401);

    let delete_no_token = app
        .as_anon()
        .delete(&app.url("/api/v1/auth/session"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_no_token.status(), 401);
}
