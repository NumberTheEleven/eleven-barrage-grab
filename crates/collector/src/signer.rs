//! WSS extraction from CDP events (auto-signer spec section 4.1)

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::broadcast;

use crate::cdp::client::CdpClient;
use crate::cdp::commands::{
    CdpCommand, CdpEvent, NetworkSetCookieParams, PageNavigateParams,
};
use crate::cdp::error::{CdpError, Result};
use crate::{SignedMaterial, SignedWssMaterial};

/// A tab attached to a CDP session.
#[derive(Debug, Clone)]
pub struct TabSession {
    pub target_id: String,
    pub session_id: String,
}

/// Returns true if the URL is a Douyin webcast push endpoint.
pub fn is_wss_push_url(url: &str) -> bool {
    url.starts_with("wss://") && url.contains("webcast/im/push")
}

/// Returns true if the URL is a Douyin webcast IM fetch (HTTP fallback) endpoint.
pub fn is_im_fetch_url(url: &str) -> bool {
    url.starts_with("https://") && url.contains("/webcast/im/fetch/")
}

/// Extract the signed endpoint material from a navigated tab.
///
/// `events` is a broadcast receiver subscribed to CdpClient events.
/// `cookies` is a map of cookie name → value (e.g., {"ttwid": "..."}).
pub async fn extract_signed_material(
    cdp: &CdpClient,
    session_id: &str,
    web_rid: &str,
    timeout: Duration,
    mut events: broadcast::Receiver<CdpEvent>,
    cookies: &HashMap<String, String>,
) -> Result<SignedMaterial> {
    // Enable Page and Network domains on this session
    cdp.send::<serde_json::Value>(
        |id| CdpCommand::PageEnable {
            id,
            session_id: Some(session_id.into()),
        },
        Duration::from_secs(2),
    )
    .await?;

    cdp.send::<serde_json::Value>(
        |id| CdpCommand::NetworkEnable {
            id,
            session_id: Some(session_id.into()),
        },
        Duration::from_secs(2),
    )
    .await?;

    // Set cookies before navigation (R-011: inject ttwid/sessionid)
    for (name, value) in cookies {
        if let Err(e) = cdp
            .send::<serde_json::Value>(
                |id| CdpCommand::NetworkSetCookie {
                    id,
                    session_id: Some(session_id.into()),
                    params: NetworkSetCookieParams {
                        name: name.clone(),
                        value: value.clone(),
                        domain: Some(".douyin.com".into()),
                        url: None,
                        path: Some("/".into()),
                    },
                },
                Duration::from_secs(2),
            )
            .await
        {
            tracing::warn!(cookie_name = %name, error = %e, "failed to set cookie");
        }
    }

    // Navigate to the live room page
    cdp.send::<serde_json::Value>(
        |id| CdpCommand::PageNavigate {
            id,
            session_id: Some(session_id.into()),
            params: PageNavigateParams {
                url: format!("https://live.douyin.com/{}", web_rid),
                referrer: Some("https://live.douyin.com/".into()),
            },
        },
        Duration::from_secs(5),
    )
    .await?;
    tracing::debug!(web_rid = %web_rid, "navigated to live room page");

    // Wait for matching endpoint event
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CdpError::NoSignedEndpointCaptured);
        }
        let evt = tokio::time::timeout(remaining, events.recv()).await;
        match evt {
            Ok(Ok(CdpEvent::RequestWillBeSent { params })) => {
                tracing::debug!(url = %params.request.url, method = %params.request.method, "cdp request observed");
                if is_wss_push_url(&params.request.url) || is_im_fetch_url(&params.request.url) {
                    let headers: HashMap<String, String> =
                        params.request.headers.into_iter().collect();
                    let material = SignedWssMaterial {
                        url: params.request.url,
                        headers,
                        expires_at: SystemTime::now() + Duration::from_secs(3600),
                    };
                    return if is_wss_push_url(&material.url) {
                        Ok(SignedMaterial::Wss(material))
                    } else {
                        Ok(SignedMaterial::HttpFetch(material))
                    };
                }
            }
            Ok(Ok(_)) => continue,    // other events, keep waiting
            Ok(Err(_)) => continue,   // broadcast lag, keep waiting
            Err(_) => return Err(CdpError::NoSignedEndpointCaptured),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_pattern_matches_douyin_push() {
        assert!(is_wss_push_url("wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/?room_id=123"));
        assert!(is_wss_push_url("wss://anything.com/webcast/im/push/v2/"));
    }

    #[test]
    fn url_pattern_matches_im_fetch_fallback() {
        assert!(is_im_fetch_url("https://live.douyin.com/webcast/im/fetch/?room_id=123"));
        assert!(is_im_fetch_url("https://www.douyin.com/webcast/im/fetch/?x=1"));
    }

    #[test]
    fn url_pattern_rejects_non_wss() {
        assert!(!is_wss_push_url("https://live.douyin.com/123"));
        assert!(!is_wss_push_url("http://example.com/webcast/im/push"));
    }

    #[test]
    fn url_pattern_rejects_wss_without_im_push() {
        // Other wss endpoints (e.g., analytics, websocket not for im messages)
        assert!(!is_wss_push_url("wss://example.com/socket"));
        assert!(!is_wss_push_url("wss://example.com/webcast/other/v2/"));
    }
}
