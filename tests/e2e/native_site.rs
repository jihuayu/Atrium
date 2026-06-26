use crate::common::TestApp;

async fn json(response: reqwest::Response) -> serde_json::Value {
    response.json::<serde_json::Value>().await.unwrap()
}

#[tokio::test]
async fn native_website_page_comment_flow() {
    let app = TestApp::start().await;

    let admin_me = app
        .as_admin()
        .get(&app.url("/api/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_me.status(), 200);
    assert_eq!(json(admin_me).await["super_admin"], true);

    let alice_me = app
        .as_alice()
        .get(&app.url("/api/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(alice_me.status(), 200);

    let website_key = "native-site";
    let origin = "https://native.example.com";
    let created = app
        .as_admin()
        .post(&app.url("/api/v1/websites"))
        .json(&serde_json::json!({
            "key": website_key,
            "name": "Native Site",
            "origins": [origin],
            "admin_user_ids": [2]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created = json(created).await;
    assert_eq!(created["key"], website_key);
    assert_eq!(created["origins"][0], origin);

    let page = app
        .as_alice()
        .put(&app.url(&format!("/api/v1/websites/{}/pages/post-1", website_key)))
        .json(&serde_json::json!({
            "title": "Post 1",
            "url": "https://native.example.com/post-1?b=2&a=1#comments",
            "metadata": { "source": "test" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    let page = json(page).await;
    assert_eq!(
        page["normalized_url"],
        "https://native.example.com/post-1?a=1&b=2"
    );

    let comment = app
        .as_bob()
        .post(&app.url(&format!(
            "/api/v1/websites/{}/pages/post-1/comments",
            website_key
        )))
        .json(&serde_json::json!({ "body": "hello **native**" }))
        .send()
        .await
        .unwrap();
    assert_eq!(comment.status(), 201);
    let comment = json(comment).await;
    let comment_id = comment["id"].as_i64().unwrap();
    assert_eq!(comment["author"]["display_name"], "bob");
    assert_eq!(comment["can_delete"], true);
    assert_eq!(comment["can_ban"], false);

    let anon_roots = app
        .as_anon()
        .get(&app.url(&format!(
            "/api/v1/websites/{}/pages/post-1/comments?parent_id=root",
            website_key
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(anon_roots.status(), 200);
    let anon_roots = json(anon_roots).await;
    assert_eq!(anon_roots["data"][0]["id"], comment_id);
    assert_eq!(anon_roots["data"][0]["can_delete"], false);
    assert_eq!(anon_roots["data"][0]["can_ban"], false);

    let admin_roots = app
        .as_alice()
        .get(&app.url(&format!(
            "/api/v1/websites/{}/pages/post-1/comments?parent_id=root",
            website_key
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_roots.status(), 200);
    let admin_roots = json(admin_roots).await;
    assert_eq!(admin_roots["data"][0]["can_delete"], true);
    assert_eq!(admin_roots["data"][0]["can_ban"], true);

    let reaction = app
        .as_alice()
        .put(&app.url(&format!(
            "/api/v1/websites/{}/comments/{}/reactions/heart",
            website_key, comment_id
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(reaction.status(), 200);
    assert_eq!(json(reaction).await["heart"], 1);

    let edited = app
        .as_bob()
        .patch(&app.url(&format!(
            "/api/v1/websites/{}/comments/{}",
            website_key, comment_id
        )))
        .json(&serde_json::json!({ "body": "edited native comment" }))
        .send()
        .await
        .unwrap();
    assert_eq!(edited.status(), 200);
    assert_eq!(json(edited).await["body"], "edited native comment");

    let referer = "https://native.example.com/post-2?z=9&a=1#section";
    let empty_current = app
        .as_anon()
        .get(&app.url("/api/v1/comments/current"))
        .header("Referer", referer)
        .send()
        .await
        .unwrap();
    assert_eq!(empty_current.status(), 200);
    assert_eq!(
        json(empty_current).await["data"].as_array().unwrap().len(),
        0
    );

    let current_comment = app
        .as_bob()
        .post(&app.url("/api/v1/comments/current"))
        .header("Referer", referer)
        .json(&serde_json::json!({ "body": "quick" }))
        .send()
        .await
        .unwrap();
    assert_eq!(current_comment.status(), 201);
    let current_comment = json(current_comment).await;
    assert_eq!(current_comment["website_key"], website_key);
    assert!(
        current_comment["page_key"]
            .as_str()
            .unwrap()
            .starts_with("url-")
    );

    let ban = app
        .as_alice()
        .post(&app.url(&format!("/api/v1/websites/{}/bans", website_key)))
        .json(&serde_json::json!({ "user_id": 3, "reason": "spam" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ban.status(), 201);
    assert_eq!(json(ban).await["data"][0]["user"]["id"], 3);

    let blocked = app
        .as_bob()
        .post(&app.url(&format!(
            "/api/v1/websites/{}/pages/post-1/comments",
            website_key
        )))
        .json(&serde_json::json!({ "body": "blocked" }))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 403);

    let unban = app
        .as_alice()
        .delete(&app.url(&format!("/api/v1/websites/{}/bans/3", website_key)))
        .send()
        .await
        .unwrap();
    assert_eq!(unban.status(), 204);
}
