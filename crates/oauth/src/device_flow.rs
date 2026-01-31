use anyhow::Result;

use crate::types::{OAuthConfig, OAuthTokens};

/// Response from the device code request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Request a device code from the provider.
pub async fn request_device_code(
    client: &reqwest::Client,
    config: &OAuthConfig,
) -> Result<DeviceCodeResponse> {
    let resp = client
        .post(&config.auth_url)
        .header("Accept", "application/json")
        .form(&[("client_id", config.client_id.as_str()), ("scope", "")])
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("device code request failed: {body}");
    }

    Ok(resp.json().await?)
}

#[derive(Debug, serde::Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    error: Option<String>,
}

/// Poll the token endpoint until the user completes the device flow.
pub async fn poll_for_token(
    client: &reqwest::Client,
    config: &OAuthConfig,
    device_code: &str,
    interval: u64,
) -> Result<OAuthTokens> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let resp = client
            .post(&config.token_url)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", config.client_id.as_str()),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        let body: TokenPollResponse = resp.json().await?;

        if let Some(token) = body.access_token {
            return Ok(OAuthTokens {
                access_token: token,
                refresh_token: None,
                expires_at: None,
            });
        }

        match body.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            },
            Some(err) => anyhow::bail!("device flow error: {err}"),
            None => anyhow::bail!("unexpected response from token endpoint"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Router, extract::Form, routing::post};

    fn test_config(auth_url: String, token_url: String) -> OAuthConfig {
        OAuthConfig {
            client_id: "test-client".into(),
            auth_url,
            token_url,
            redirect_uri: String::new(),
            scopes: vec![],
            extra_auth_params: vec![],
            device_flow: true,
        }
    }

    /// Start a mock HTTP server and return its base URL.
    async fn start_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn device_code_response_deserialize() {
        let json = r#"{
            "device_code": "dc_123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device"
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "dc_123");
        assert_eq!(resp.user_code, "ABCD-1234");
        assert_eq!(resp.verification_uri, "https://github.com/login/device");
        assert_eq!(resp.interval, 5); // default
    }

    #[test]
    fn device_code_response_with_interval() {
        let json = r#"{
            "device_code": "dc",
            "user_code": "CODE",
            "verification_uri": "https://example.com",
            "interval": 10
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.interval, 10);
    }

    #[test]
    fn device_code_response_serialize_roundtrip() {
        let resp = DeviceCodeResponse {
            device_code: "dc_abc".into(),
            user_code: "WXYZ-1234".into(),
            verification_uri: "https://example.com/device".into(),
            interval: 8,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: DeviceCodeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.device_code, "dc_abc");
        assert_eq!(back.user_code, "WXYZ-1234");
        assert_eq!(back.interval, 8);
    }

    #[tokio::test]
    async fn request_device_code_success() {
        let app = Router::new().route(
            "/device/code",
            post(|| async {
                axum::Json(serde_json::json!({
                    "device_code": "mock_dc",
                    "user_code": "TEST-CODE",
                    "verification_uri": "https://example.com/device",
                    "interval": 1
                }))
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(format!("{base}/device/code"), String::new());

        let client = reqwest::Client::new();
        let resp = request_device_code(&client, &config).await.unwrap();
        assert_eq!(resp.device_code, "mock_dc");
        assert_eq!(resp.user_code, "TEST-CODE");
        assert_eq!(resp.verification_uri, "https://example.com/device");
        assert_eq!(resp.interval, 1);
    }

    #[tokio::test]
    async fn request_device_code_server_error() {
        let app = Router::new().route(
            "/device/code",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
        let base = start_mock(app).await;
        let config = test_config(format!("{base}/device/code"), String::new());

        let client = reqwest::Client::new();
        let err = request_device_code(&client, &config).await.unwrap_err();
        assert!(err.to_string().contains("device code request failed"));
    }

    #[tokio::test]
    async fn poll_for_token_immediate_success() {
        let app = Router::new().route(
            "/token",
            post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "ghp_mock_token"
                }))
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let tokens = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap();
        assert_eq!(tokens.access_token, "ghp_mock_token");
        assert!(tokens.refresh_token.is_none());
    }

    #[tokio::test]
    async fn poll_for_token_pending_then_success() {
        // Return "authorization_pending" once, then success
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = call_count.clone();

        let app = Router::new().route(
            "/token",
            post(move |_body: Form<Vec<(String, String)>>| {
                let counter = counter.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        axum::Json(serde_json::json!({"error": "authorization_pending"}))
                    } else {
                        axum::Json(serde_json::json!({"access_token": "ghp_success"}))
                    }
                }
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let tokens = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap();
        assert_eq!(tokens.access_token, "ghp_success");
        assert!(call_count.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn poll_for_token_access_denied_error() {
        let app = Router::new().route(
            "/token",
            post(|| async { axum::Json(serde_json::json!({"error": "access_denied"})) }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap_err();
        assert!(err.to_string().contains("access_denied"));
    }

    #[tokio::test]
    async fn poll_for_token_unexpected_response() {
        let app = Router::new().route(
            "/token",
            post(|| async { axum::Json(serde_json::json!({})) }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap_err();
        assert!(err.to_string().contains("unexpected response"));
    }
}
