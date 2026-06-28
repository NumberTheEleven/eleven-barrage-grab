//! WSS extraction from CDP events (auto-signer spec section 4.1)

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::broadcast;

use crate::cdp::client::CdpClient;
use crate::cdp::commands::{
    CdpCommand, CdpEvent, NavigateParams, NetworkEnableParams, PageEnableParams,
};
use crate::cdp::error::{CdpError, Result};
use crate::SignedWssMaterial;

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

/// Extract the signed WSS material from a navigated tab.
///
/// `events` is a broadcast receiver subscribed to CdpClient events.
pub async fn extract_wss(
    cdp: &CdpClient,
    session_id: &str,
    web_rid: &str,
    timeout: Duration,
    mut events: broadcast::Receiver<CdpEvent>,
) -> Result<SignedWssMaterial> {
    // Enable Page and Network domains on this session
    cdp.send::<()>(
        |id| CdpCommand::PageEnable {
            id,
            params: PageEnableParams {
                session_id: Some(session_id.into()),
            },
        },
        Duration::from_secs(2),
    )
    .await?;

    cdp.send::<()>(
        |id| CdpCommand::NetworkEnable {
            id,
            params: NetworkEnableParams {
                session_id: Some(session_id.into()),
            },
        },
        Duration::from_secs(2),
    )
    .await?;

    // Navigate to the live room page
    cdp.send::<()>(
        |id| CdpCommand::PageNavigate {
            id,
            params: NavigateParams {
                url: format!("https://live.douyin.com/{}", web_rid),
                session_id: Some(session_id.into()),
                referrer: Some("https://live.douyin.com/".into()),
            },
        },
        Duration::from_secs(5),
    )
    .await?;

    // Wait for matching WSS event
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CdpError::Timeout(timeout));
        }
        let evt = tokio::time::timeout(remaining, events.recv()).await;
        match evt {
            Ok(Ok(CdpEvent::RequestWillBeSent { params })) => {
                if is_wss_push_url(&params.request.url) {
                    let headers: HashMap<String, String> =
                        params.request.headers.into_iter().collect();
                    return Ok(SignedWssMaterial {
                        url: params.request.url,
                        headers,
                        expires_at: SystemTime::now() + Duration::from_secs(3600),
                    });
                }
            }
            Ok(Ok(_)) => continue,    // other events, keep waiting
            Ok(Err(_)) => continue,   // broadcast lag, keep waiting
            Err(_) => return Err(CdpError::Timeout(timeout)),
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
