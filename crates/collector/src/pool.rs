//! Browser pool with round-robin scheduling (auto-signer spec section 2)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::browser::{Browser, BrowserConfig};
use crate::cdp::client::CdpClient;
use crate::cdp::commands::{CdpCommand, CdpEvent, NetworkGetAllCookiesResult};
use crate::signer::{extract_signed_material, TabSession};
use crate::{SignedMaterial};

#[derive(Debug, Clone)]
pub struct BrowserPoolConfig {
    pub pool_size: usize,
    pub max_concurrent_per_browser: usize,
    pub sign_timeout: Duration,
    pub health_check_interval: Duration,
    pub edge_path: PathBuf,
    pub user_data_dir_template: String,
    pub extra_args: Vec<String>,
    pub cdp_port_base: u16,
    /// Auth cookies to inject before signing (name → value, e.g. ttwid, sessionid)
    pub auth_cookies: HashMap<String, String>,
}

impl Default for BrowserPoolConfig {
    fn default() -> Self {
        Self {
            pool_size: 3,
            max_concurrent_per_browser: 2,
            sign_timeout: Duration::from_secs(10),
            health_check_interval: Duration::from_secs(30),
            edge_path: PathBuf::from(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            ),
            user_data_dir_template: "./data/browser-{id}".into(),
            extra_args: vec![],
            cdp_port_base: 9222,
            auth_cookies: HashMap::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum PoolError {
    #[error("pool busy: all browsers saturated")]
    Busy,
    #[error("sign failed: {0}")]
    Sign(String),
    #[error("browser failed: {0}")]
    Browser(String),
}

impl PoolError {
    /// Returns true if this error represents a timeout during signing.
    pub fn is_timeout(&self) -> bool {
        matches!(self, PoolError::Sign(msg) if msg.contains("timed out"))
    }

    /// Returns true if the page navigated but no signed endpoint was captured.
    pub fn is_no_signed_endpoint_captured(&self) -> bool {
        matches!(self, PoolError::Sign(msg) if msg.contains("NoSignedEndpointCaptured"))
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BrowserHealth {
    pub id: usize,
    pub state: String,
    pub last_sign_age_ms: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct PoolHealth {
    pub size: usize,
    pub ready: usize,
    pub busy: usize,
    pub dead: usize,
    pub browsers: Vec<BrowserHealth>,
}

pub struct BrowserHandle {
    pub id: usize,
    pub semaphore: Arc<Semaphore>,
    pub(crate) inner: Arc<BrowserInner>,
}

pub struct BrowserInner {
    pub browser: tokio::sync::Mutex<Browser>,
    pub cdp: tokio::sync::RwLock<CdpClient>,
    pub last_sign: tokio::sync::Mutex<Option<Instant>>,
}

pub struct BrowserPool {
    pub(crate) browsers: Vec<BrowserHandle>,
    next_index: AtomicUsize,
    pub(crate) config: BrowserPoolConfig,
    cookies: HashMap<String, String>,
}

impl BrowserPool {
    /// Spawn the pool and start health check loop.
    /// Returns `Arc<Self>` so callers can share the pool across tasks/clones.
    pub async fn start(config: BrowserPoolConfig) -> Result<Arc<Self>> {
        let mut browsers = Vec::with_capacity(config.pool_size);
        for i in 0..config.pool_size {
            let user_data_dir = config
                .user_data_dir_template
                .replace("{id}", &i.to_string());
            std::fs::create_dir_all(&user_data_dir).ok();

            let cdp_port = config.cdp_port_base + i as u16;
            let browser_config = BrowserConfig {
                edge_path: config.edge_path.clone(),
                user_data_dir: PathBuf::from(user_data_dir),
                extra_args: config.extra_args.clone(),
                cdp_port,
            };

            let mut browser = Browser::spawn(browser_config).context("spawn browser")?;
            let ws_url = browser.discover_cdp_url().await.context("discover cdp")?;
            let (cdp, _global_events) =
                CdpClient::connect(&ws_url).await.context("connect cdp")?;

            browsers.push(BrowserHandle {
                id: i,
                semaphore: Arc::new(Semaphore::new(config.max_concurrent_per_browser)),
                inner: Arc::new(BrowserInner {
                    browser: tokio::sync::Mutex::new(browser),
                    cdp: tokio::sync::RwLock::new(cdp),
                    last_sign: tokio::sync::Mutex::new(None),
                }),
            });
        }

        let mut cookies = config.auth_cookies.clone();

        // 如果配置里没有提供 ttwid/sessionid，让浏览器先访问一次抖音首页，
        // 从浏览器 profile 中把登录态 cookie 捞出来。
        if cookies.is_empty() {
            if let Some(handle) = browsers.first() {
                let cdp = handle.inner.cdp.read().await;
                match warmup_auth_cookies(
                    &cdp,
                    std::time::Duration::from_secs(20),
                )
                .await
                {
                    Ok(warmed) if !warmed.is_empty() => {
                        tracing::info!(
                            cookie_names = ?warmed.keys().collect::<Vec<_>>(),
                            "warmed auth cookies from browser"
                        );
                        cookies = warmed;
                    }
                    Ok(_) => {
                        tracing::warn!(
                            "browser warmup completed but no ttwid/sessionid cookie found"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to warm auth cookies from browser");
                    }
                }
            }
        }

        let pool = Arc::new(Self {
            browsers,
            next_index: AtomicUsize::new(0),
            config,
            cookies,
        });

        // Start health check task
        let health_pool = pool.clone();
        tokio::spawn(async move {
            health_check_loop(health_pool).await;
        });

        Ok(pool)
    }

    /// Sign a single web_rid using the pool's stored auth cookies. Round-robin across browsers.
    pub async fn sign(&self, web_rid: &str) -> Result<SignedMaterial, PoolError> {
        self.sign_with_cookies(web_rid, &self.cookies).await
    }

    /// Sign with explicit cookies (allows overriding pool's stored cookies).
    pub async fn sign_with_cookies(
        &self,
        web_rid: &str,
        cookies: &HashMap<String, String>,
    ) -> Result<SignedMaterial, PoolError> {
        for _ in 0..self.browsers.len() {
            let idx = self.next_index.fetch_add(1, Ordering::Relaxed) % self.browsers.len();
            let handle = &self.browsers[idx];

            let permit = match handle.semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let result = self.sign_with_handle(handle, web_rid, cookies).await;
            drop(permit);
            return result;
        }
        Err(PoolError::Busy)
    }

    async fn sign_with_handle(
        &self,
        handle: &BrowserHandle,
        web_rid: &str,
        cookies: &std::collections::HashMap<String, String>,
    ) -> Result<SignedMaterial, PoolError> {
        *handle.inner.last_sign.lock().await = Some(Instant::now());

        // Hold a read lock on cdp for the entire signing workflow.
        // The write lock is only taken briefly during health-check restarts.
        let cdp = handle.inner.cdp.read().await;

        // Acquire a fresh tab
        let tab = acquire_tab(&cdp)
            .await
            .map_err(|e| PoolError::Sign(e.to_string()))?;

        let events = cdp.subscribe_session(&tab.session_id);

        let result = extract_signed_material(
            &cdp,
            &tab.session_id,
            web_rid,
            self.config.sign_timeout,
            events,
            cookies,
        )
        .await;

        // Cleanup tab regardless of result
        let _ = close_tab(&cdp, &tab.target_id).await;

        result.map_err(|e| PoolError::Sign(e.to_string()))
    }

    pub async fn health(&self) -> PoolHealth {
        let mut browsers = Vec::with_capacity(self.browsers.len());
        let mut ready = 0;
        let mut busy = 0;
        let mut dead = 0;
        for h in &self.browsers {
            let state = {
                let mut browser = h.inner.browser.lock().await;
                if !browser.is_alive() {
                    dead += 1;
                    "Dead".to_string()
                } else if h.semaphore.available_permits() == 0 {
                    busy += 1;
                    "Signing".to_string()
                } else {
                    ready += 1;
                    "Idle".to_string()
                }
            };
            let age_ms = h
                .inner
                .last_sign
                .lock()
                .await
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(u64::MAX);
            browsers.push(BrowserHealth {
                id: h.id,
                state,
                last_sign_age_ms: age_ms,
            });
        }
        PoolHealth {
            size: self.browsers.len(),
            ready,
            busy,
            dead,
            browsers,
        }
    }
}

async fn acquire_tab(cdp: &CdpClient) -> Result<TabSession, crate::cdp::error::CdpError> {
    use crate::cdp::commands::{CreateTargetParams, AttachToTargetParams, AttachToTargetResult};

    let target: crate::cdp::commands::CreateTargetResult = cdp
        .send(
            |id| CdpCommand::CreateTarget {
                id,
                params: CreateTargetParams {
                    url: "about:blank".into(),
                },
            },
            Duration::from_secs(3),
        )
        .await?;

    let attach: AttachToTargetResult = cdp
        .send(
            |id| CdpCommand::AttachToTarget {
                id,
                params: AttachToTargetParams {
                    target_id: target.target_id.clone(),
                    flatten: Some(true),
                },
            },
            Duration::from_secs(3),
        )
        .await?;

    Ok(TabSession {
        target_id: target.target_id,
        session_id: attach.session_id,
    })
}

async fn close_tab(cdp: &CdpClient, target_id: &str) -> Result<(), crate::cdp::error::CdpError> {
    use crate::cdp::commands::CloseTargetParams;
    let _: serde_json::Value = cdp
        .send(
            |id| CdpCommand::CloseTarget {
                id,
                params: CloseTargetParams {
                    target_id: target_id.into(),
                },
            },
            Duration::from_secs(2),
        )
        .await?;
    Ok(())
}

async fn warmup_auth_cookies(
    cdp: &CdpClient,
    timeout: Duration,
) -> Result<HashMap<String, String>, crate::cdp::error::CdpError> {
    use crate::cdp::commands::{NetworkGetCookiesParams, PageNavigateParams};

    let tab = acquire_tab(cdp).await?;
    let mut events = cdp.subscribe_session(&tab.session_id);

    // Enable Page / Network on the warmup tab
    cdp.send::<serde_json::Value>(
        |id| CdpCommand::PageEnable {
            id,
            session_id: Some(tab.session_id.clone()),
        },
        Duration::from_secs(2),
    )
    .await?;

    cdp.send::<serde_json::Value>(
        |id| CdpCommand::NetworkEnable {
            id,
            session_id: Some(tab.session_id.clone()),
        },
        Duration::from_secs(2),
    )
    .await?;

    // Navigate to Douyin landing page; this triggers ttwid/sessionid generation.
    cdp.send::<serde_json::Value>(
        |id| CdpCommand::PageNavigate {
            id,
            session_id: Some(tab.session_id.clone()),
            params: PageNavigateParams {
                url: "https://www.douyin.com".into(),
                referrer: Some("https://www.douyin.com/".into()),
            },
        },
        Duration::from_secs(5),
    )
    .await?;

    // Wait for the page load event (or until timeout).
    let _ = tokio::time::timeout(timeout, async {
        while let Ok(evt) = events.recv().await {
            if matches!(evt, CdpEvent::LoadEventFired { .. }) {
                break;
            }
        }
    })
    .await;

    // Pull cookies relevant to Douyin domains from the current session.
    let result: NetworkGetAllCookiesResult = cdp
        .send(
            |id| CdpCommand::NetworkGetCookies {
                id,
                session_id: Some(tab.session_id.clone()),
                params: NetworkGetCookiesParams {
                    urls: vec![
                        "https://www.douyin.com".into(),
                        "https://live.douyin.com".into(),
                    ],
                },
            },
            Duration::from_secs(5),
        )
        .await?;

    let mut cookies = HashMap::new();
    for cookie in result.cookies {
        let domain_ok = cookie.domain == ".douyin.com"
            || cookie.domain == "douyin.com"
            || cookie.domain.ends_with(".douyin.com");
        if domain_ok && (cookie.name == "ttwid" || cookie.name == "sessionid") {
            tracing::debug!(
                name = %cookie.name,
                domain = %cookie.domain,
                "found auth cookie from browser warmup"
            );
            cookies.insert(cookie.name, cookie.value);
        }
    }

    // Best-effort cleanup.
    let _ = close_tab(cdp, &tab.target_id).await;
    Ok(cookies)
}

async fn health_check_loop(pool: Arc<BrowserPool>) {
    let mut interval = tokio::time::interval(pool.config.health_check_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for h in &pool.browsers {
            let mut browser = h.inner.browser.lock().await;
            if !browser.is_alive() {
                tracing::warn!(browser_id = h.id, "browser dead, restarting");
                let _ = browser.kill();
                let browser_config = BrowserConfig {
                    edge_path: pool.config.edge_path.clone(),
                    user_data_dir: PathBuf::from(
                        pool.config
                            .user_data_dir_template
                            .replace("{id}", &h.id.to_string()),
                    ),
                    extra_args: pool.config.extra_args.clone(),
                    cdp_port: pool.config.cdp_port_base + h.id as u16,
                };
                match Browser::spawn(browser_config) {
                    Ok(mut new_browser) => match new_browser.discover_cdp_url().await {
                        Ok(_) => {
                            let ws_url = new_browser.cdp_ws_url.clone();
                            match CdpClient::connect(&ws_url).await {
                                Ok((new_cdp, _global_events)) => {
                                    let mut cdp = h.inner.cdp.write().await;
                                    *cdp = new_cdp;
                                    *browser = new_browser;
                                    tracing::info!(browser_id = h.id, "browser restarted");
                                }
                                Err(e) => {
                                    tracing::error!(
                                        browser_id = h.id,
                                        error = %e,
                                        "restart CDP connect failed"
                                    );
                                    let _ = new_browser.kill();
                                }
                            }
                        }
                        Err(e) => tracing::error!(
                            browser_id = h.id,
                            error = %e,
                            "restart CDP discovery failed"
                        ),
                    },
                    Err(e) => tracing::error!(
                        browser_id = h.id,
                        error = %e,
                        "browser restart spawn failed"
                    ),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::TryAcquireError;

    #[test]
    fn pool_health_serializes_to_json() {
        let health = PoolHealth {
            size: 3,
            ready: 2,
            busy: 1,
            dead: 0,
            browsers: vec![
                BrowserHealth { id: 0, state: "Idle".into(), last_sign_age_ms: 100 },
                BrowserHealth { id: 1, state: "Idle".into(), last_sign_age_ms: 50 },
                BrowserHealth { id: 2, state: "Signing".into(), last_sign_age_ms: 0 },
            ],
        };
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("\"size\":3"));
        assert!(json.contains("\"state\":\"Idle\""));
    }

    #[test]
    fn round_robin_counter_cycles_correctly() {
        let counter = AtomicUsize::new(0);
        let pool_size = 3;
        let indices: Vec<usize> = (0..6)
            .map(|_| counter.fetch_add(1, Ordering::Relaxed) % pool_size)
            .collect();
        assert_eq!(indices, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn semaphore_exhaustion_blocks_subsequent_acquire() {
        let sem = Arc::new(Semaphore::new(1));
        let _p = sem.clone().try_acquire_owned().unwrap();
        let result = sem.clone().try_acquire_owned();
        assert!(matches!(result, Err(TryAcquireError::NoPermits)));
    }

    #[test]
    fn pool_error_display_includes_busy() {
        assert_eq!(PoolError::Busy.to_string(), "pool busy: all browsers saturated");
    }
}