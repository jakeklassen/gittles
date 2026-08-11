//! The GitHub OAuth device flow.
//!
//! Deliberately loop-free: core hands back one poll at a time so the caller can
//! drive the wait on whatever timer it already owns (gpui's executor, a tokio
//! interval, a test's fake clock) instead of core assuming a runtime.

use anyhow::Result;
use serde::Deserialize;
use std::time::Duration;

use crate::github::{USER_AGENT, fail};

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const CLIENT_ID: &str = "Ov23ligv9nNkVGihgxUF";
const SCOPES: &str = "read:user repo";
/// GitHub's documented penalty for polling too fast.
const SLOW_DOWN_BUMP: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    /// Shown to the user; they type this into the verification page.
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds GitHub asks us to wait between polls.
    pub interval: u64,
    pub expires_in: u64,
}

impl DeviceCode {
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval)
    }

    pub fn expires_in(&self) -> Duration {
        Duration::from_secs(self.expires_in)
    }
}

/// What one poll of the token endpoint means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Authorized(String),
    /// The user has not finished authorizing yet — poll again.
    Pending,
    /// We polled too fast; widen the interval and poll again.
    SlowDown,
}

/// Both shapes come back on the same endpoint, so both fields are optional.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(USER_AGENT).build()?)
}

/// Step one: ask GitHub for a code to show the user.
pub async fn request_device_code() -> Result<DeviceCode> {
    let response = client()?
        .post(DEVICE_CODE_URL)
        .header("accept", "application/json")
        .json(&serde_json::json!({ "client_id": CLIENT_ID, "scope": SCOPES }))
        .send()
        .await?;

    let status = response.status().as_u16();
    let body = response.text().await?;

    if status != 200 {
        return Err(fail("device code request", status, &body));
    }

    Ok(serde_json::from_str(&body)?)
}

/// Step two, once per interval, until it returns [`Poll::Authorized`] or the
/// device code expires.
pub async fn poll_once(device_code: &str) -> Result<Poll> {
    let body = client()?
        .post(ACCESS_TOKEN_URL)
        .header("accept", "application/json")
        .json(&serde_json::json!({
            "client_id": CLIENT_ID,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        }))
        .send()
        .await?
        .text()
        .await?;

    classify(&body)
}

/// The pure half of a poll: response body in, meaning out.
fn classify(body: &str) -> Result<Poll> {
    let response: TokenResponse = serde_json::from_str(body)?;

    if let Some(token) = response.access_token
        && !token.is_empty()
    {
        return Ok(Poll::Authorized(token));
    }

    match response.error.as_deref() {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown),
        Some(error) => Err(anyhow::anyhow!("authorization failed: {error}")),
        None => Err(anyhow::anyhow!("authorization failed: unknown error")),
    }
}

/// Back-off policy, split out so the caller's loop stays trivial.
pub fn next_interval(current: Duration, outcome: &Poll) -> Duration {
    match outcome {
        Poll::SlowDown => current + SLOW_DOWN_BUMP,
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_device_code_response() {
        let device: DeviceCode = serde_json::from_str(
            r#"{
                "device_code": "3584d83530557fdd1f46af8289938c8ef79f9dc5",
                "user_code": "WDJB-MJHT",
                "verification_uri": "https://github.com/login/device",
                "interval": 5,
                "expires_in": 900
            }"#,
        )
        .unwrap();

        assert_eq!(device.user_code, "WDJB-MJHT");
        assert_eq!(device.interval(), Duration::from_secs(5));
        assert_eq!(device.expires_in(), Duration::from_secs(900));
    }

    #[test]
    fn a_token_means_authorized() {
        let outcome = classify(r#"{"access_token":"gho_secret","token_type":"bearer"}"#).unwrap();
        assert_eq!(outcome, Poll::Authorized("gho_secret".into()));
    }

    #[test]
    fn authorization_pending_means_keep_waiting() {
        let outcome = classify(r#"{"error":"authorization_pending"}"#).unwrap();
        assert_eq!(outcome, Poll::Pending);
    }

    #[test]
    fn slow_down_means_keep_waiting_longer() {
        let outcome = classify(r#"{"error":"slow_down"}"#).unwrap();
        assert_eq!(outcome, Poll::SlowDown);
        assert_eq!(
            next_interval(Duration::from_secs(5), &outcome),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn only_slow_down_widens_the_interval() {
        let five = Duration::from_secs(5);
        assert_eq!(next_interval(five, &Poll::Pending), five);
        assert_eq!(
            next_interval(five, &Poll::Authorized("gho_secret".into())),
            five
        );
    }

    #[test]
    fn an_empty_token_is_not_an_authorization() {
        // GitHub returning `""` must not be mistaken for success.
        let error = classify(r#"{"access_token":""}"#).unwrap_err();
        assert_eq!(error.to_string(), "authorization failed: unknown error");
    }

    #[test]
    fn other_errors_surface_verbatim() {
        let error = classify(r#"{"error":"expired_token"}"#).unwrap_err();
        assert_eq!(error.to_string(), "authorization failed: expired_token");
    }

    #[test]
    fn a_bodyless_response_is_an_error_not_a_panic() {
        assert!(classify("").is_err());
    }
}
