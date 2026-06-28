//! Browser pool with round-robin scheduling (auto-signer spec section 2)

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
use crate::cdp::commands::{CdpCommand, CdpEvent};
use crate::signer::{extract_wss, TabSession};
use crate::SignedWssMaterial;

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
    pub cdp: CdpClient,
    pub last_sign: tokio::sync::Mutex<Option<Instant>>,
}

pub struct BrowserPool {
    pub(crate) browsers: Vec<BrowserHandle>,
    next_index: AtomicUsize,
    pub(crate) config: BrowserPoolConfig,
}

impl BrowserPool {
    /// Spawn the pool and start health check loop.
    pub async fn start(config: BrowserPoolConfig) -> Result<Self> {
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
                    cdp,
                    last_sign: tokio::sync::Mutex::new(None),
                }),
            });
        }

        let pool = Self {
            browsers,
            next_index: AtomicUsize::new(0),
            config,
        };

        // Start health check task
        let pool_arc = Arc::new(pool);
        let health_pool = pool_arc.clone();
        tokio::spawn(async move {
            health_check_loop(health_pool).await;
        });

        // Unwrap the Arc and return the pool
        Arc::try_unwrap(pool_arc).map_err(|_| anyhow::anyhow!("pool has multiple references"))
    }

    /// Sign a single web_rid. Round-robin across browsers.
    pub async fn sign(&self, web_rid: &str) -> Result<SignedWssMaterial, PoolError> {
        for _ in 0..self.browsers.len() {
            let idx = self.next_index.fetch_add(1, Ordering::Relaxed) % self.browsers.len();
            let handle = &self.browsers[idx];

            let permit = match handle.semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let result = self.sign_with_handle(handle, web_rid).await;
            drop(permit);
            return result;
        }
        Err(PoolError::Busy)
    }

    async fn sign_with_handle(
        &self,
        handle: &BrowserHandle,
        web_rid: &str,
    ) -> Result<SignedWssMaterial, PoolError> {
        *handle.inner.last_sign.lock().await = Some(Instant::now());

        // Acquire a fresh tab
        let tab = acquire_tab(&handle.inner.cdp)
            .await
            .map_err(|e| PoolError::Sign(e.to_string()))?;

        let events = handle.inner.cdp.subscribe_session(&tab.session_id);

        let result = extract_wss(
            &handle.inner.cdp,
            &tab.session_id,
            web_rid,
            self.config.sign_timeout,
            events,
        )
        .await;

        // Cleanup tab regardless of result
        let _ = close_tab(&handle.inner.cdp, &tab.target_id).await;

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
    use crate::cdp::commands::CreateTargetParams;

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

    // Subscribe to attachedToTarget
    let mut events = cdp.subscribe_events();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(CdpEvent::AttachedToTarget { params })) => {
                if params.target_info.target_id == target.target_id {
                    return Ok(TabSession {
                        target_id: target.target_id,
                        session_id: params.session_id,
                    });
                }
            }
            _ => continue,
        }
    }
    Err(crate::cdp::error::CdpError::Timeout(Duration::from_secs(3)))
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
                            tracing::info!(browser_id = h.id, "browser restarted");
                            *browser = new_browser;
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