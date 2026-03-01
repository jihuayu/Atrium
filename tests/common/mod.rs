use reqwest::RequestBuilder;

pub mod fixtures;

pub struct TestApp {
    pub base_url: String,
    pub bypass_secret: String,
    _guard: TestGuard,
}

enum TestGuard {
    #[cfg(feature = "server")]
    InProcess {
        _handle: tokio::task::AbortHandle,
        _db_file: tempfile::TempPath,
    },
    External,
}

pub struct AuthClient {
    client: reqwest::Client,
    auth: Option<String>,
}

impl TestApp {
    pub async fn start() -> Self {
        match std::env::var("ATRIUM_TEST_BASE_URL")
            .or_else(|_| std::env::var("XTALK_TEST_BASE_URL"))
        {
            Ok(url) => {
                let secret = std::env::var("ATRIUM_TEST_BYPASS_SECRET")
                    .or_else(|_| std::env::var("XTALK_TEST_BYPASS_SECRET"))
                    .expect(
                        "ATRIUM_TEST_BYPASS_SECRET or XTALK_TEST_BYPASS_SECRET must be set for external target",
                    );
                Self {
                    base_url: url,
                    bypass_secret: secret,
                    _guard: TestGuard::External,
                }
            }
            Err(_) => {
                #[cfg(feature = "server")]
                {
                    return Self::spawn_server().await;
                }

                #[cfg(not(feature = "server"))]
                {
                    panic!(
                        "ATRIUM_TEST_BASE_URL or XTALK_TEST_BASE_URL is required when server feature is disabled"
                    );
                }
            }
        }
    }

    #[cfg(feature = "server")]
    async fn spawn_server() -> Self {
        let secret = std::env::var("ATRIUM_TEST_BYPASS_SECRET")
            .or_else(|_| std::env::var("XTALK_TEST_BYPASS_SECRET"))
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "atrium-test-bypass-secret".to_string());
        // SAFETY: test helper writes process env in a controlled test context.
        unsafe {
            std::env::set_var("ATRIUM_TEST_BYPASS_SECRET", &secret);
            std::env::set_var("XTALK_TEST_BYPASS_SECRET", &secret);
        }

        let db_file = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let db_path = db_file.to_string_lossy().replace('\\', "/");
        let db_url = format!("sqlite://{}", db_path);

        let app = atrium::platform::server::build_app(
            &db_url,
            "http://localhost".to_string(),
            3600,
            1000,
            60,
            b"test-jwt-secret-at-least-32-bytes!!".to_vec(),
            None,
            None,
        )
        .await
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self {
            base_url: format!("http://{}", addr),
            bypass_secret: secret,
            _guard: TestGuard::InProcess {
                _handle: handle.abort_handle(),
                _db_file: db_file,
            },
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub fn as_user(&self, id: i64, login: &str) -> AuthClient {
        AuthClient::new(format!(
            "testuser {}:{}:{}:{}@test.com",
            self.bypass_secret, id, login, login
        ))
    }

    pub fn as_admin(&self) -> AuthClient {
        self.as_user(1, "admin")
    }

    pub fn as_alice(&self) -> AuthClient {
        self.as_user(2, "alice")
    }

    pub fn as_bob(&self) -> AuthClient {
        self.as_user(3, "bob")
    }

    pub fn as_anon(&self) -> AuthClient {
        AuthClient::new_anon()
    }
}

impl AuthClient {
    pub fn new(auth_header: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            auth: Some(auth_header),
        }
    }

    pub fn new_anon() -> Self {
        Self {
            client: reqwest::Client::new(),
            auth: None,
        }
    }

    pub fn get(&self, url: &str) -> RequestBuilder {
        self.req(|c, u| c.get(u), url)
    }

    pub fn post(&self, url: &str) -> RequestBuilder {
        self.req(|c, u| c.post(u), url)
    }

    pub fn patch(&self, url: &str) -> RequestBuilder {
        self.req(|c, u| c.patch(u), url)
    }

    pub fn delete(&self, url: &str) -> RequestBuilder {
        self.req(|c, u| c.delete(u), url)
    }

    fn req(
        &self,
        f: impl Fn(&reqwest::Client, &str) -> RequestBuilder,
        url: &str,
    ) -> RequestBuilder {
        let builder = f(&self.client, url);
        match &self.auth {
            Some(auth) => builder.header("Authorization", auth),
            None => builder,
        }
    }
}
