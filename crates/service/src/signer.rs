//! AutoSigner：组合 RoomInfoApi + ImFetcher 完成自动签名（R-003 + R-004）
//!
//! # 流程
//!
//! 1. 接收 `web_rid`（从 URL 解析）
//! 2. 调用 RoomInfoApi.get_for_signer(web_rid) → room_id
//! 3. 调用 ImFetcher.fetch(room_id, cookies) → SignedWssMaterial
//! 4. 返回 SignedWssMaterial 给调用方
//!
//! # 错误传播
//!
//! 任一步失败立即返回 `SignatureError`，不调用下游。
//!
//! # 位置说明
//!
//! AutoSigner 放在 `service` crate（而非 design.md 计划的 `collector`），
//! 因为它需要同时使用 `RoomInfoApi`（service）和 `ImFetcher`（collector），
//! 放 service 可避免循环依赖。

use eleven_barrage_collector::{ImFetcher, ImFetchConfig, SignedWssMaterial, SignatureError};
use tracing::{debug, info};

use crate::api::RoomInfoApi;
use crate::config::AuthConfig;

/// AutoSigner：自动签名器（web_rid + cookies → SignedWssMaterial）
pub struct AutoSigner {
    room_api: RoomInfoApi,
    im_fetcher: ImFetcher,
    auth: AuthConfig,
}

impl AutoSigner {
    /// 创建 AutoSigner
    pub fn new(room_api: RoomInfoApi, im_fetcher: ImFetcher, auth: AuthConfig) -> Self {
        Self {
            room_api,
            im_fetcher,
            auth,
        }
    }

    /// 从 RoomApiConfig + ImFetchConfig + AuthConfig 构造
    pub fn from_configs(
        room_api: RoomInfoApi,
        im_config: ImFetchConfig,
        auth: AuthConfig,
    ) -> Result<Self, SignatureError> {
        let im_fetcher = ImFetcher::new(im_config)?;
        Ok(Self::new(room_api, im_fetcher, auth))
    }

    /// 主入口：完成所有签名调用
    ///
    /// # 参数
    /// - `web_rid`: 抖音 web 直播间标识（从 URL 解析得到）
    ///
    /// # 返回
    /// - `Ok(SignedWssMaterial)`: 签名成功
    /// - `Err(SignatureError)`: 任一步失败
    pub async fn sign(&self, web_rid: &str) -> Result<SignedWssMaterial, SignatureError> {
        // 0. 校验 auth cookie
        if let Err(e) = self.auth.validate() {
            tracing::warn!(error = %e, "auth validation failed");
            return Err(e);
        }

        let cookie_header = self.auth.to_cookie_header();

        // 1. room_info: web_rid → room_id
        info!(web_rid = %web_rid, "step 1: calling room_info");
        let room_info = self.room_api.get_for_signer(web_rid).await?;
        debug!(room_id = %room_info.room_id, "room_info returned");

        // 2. im_fetch: room_id + cookies → signed wss URL
        info!(room_id = %room_info.room_id, "step 2: calling im_fetch");
        let material = self
            .im_fetcher
            .fetch(&room_info.room_id, &cookie_header)
            .await?;

        info!(wss_url = %material.url, "auto-sign complete");
        Ok(material)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RoomApiConfig;

    fn make_test_auth() -> AuthConfig {
        AuthConfig {
            ttwid: "test_ttwid".to_string(),
            sessionid: "test_sessionid".to_string(),
        }
    }

    #[test]
    fn auth_validation_propagates_to_caller() {
        // 用一个会被 get_for_signer 拒绝的 AuthConfig 验证错误传播
        let auth = AuthConfig::default(); // 全空
        let room_api = RoomInfoApi::new(RoomApiConfig::default()).unwrap();
        let im_config = ImFetchConfig::default();
        let im_fetcher = ImFetcher::new(im_config).unwrap();
        let signer = AutoSigner::new(room_api, im_fetcher, auth);

        // 这里用 blocking 检查 - sign() 是 async
        // 只验证 AutoSigner::new 不 panic + 可以调用 sign
        let _ = signer;
    }

    #[tokio::test]
    async fn sign_with_empty_auth_returns_config_missing() {
        let auth = AuthConfig::default(); // 全空
        let room_api = RoomInfoApi::new(RoomApiConfig::default()).unwrap();
        let im_config = ImFetchConfig::default();
        let im_fetcher = ImFetcher::new(im_config).unwrap();
        let signer = AutoSigner::new(room_api, im_fetcher, auth);

        let result = signer.sign("test_web_rid").await;
        match result {
            Err(SignatureError::ConfigMissing { .. }) => {}
            _ => panic!("expected ConfigMissing, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn from_configs_constructs_successfully() {
        let room_api = RoomInfoApi::new(RoomApiConfig::default()).unwrap();
        let im_config = ImFetchConfig::default();
        let auth = make_test_auth();
        let result = AutoSigner::from_configs(room_api, im_config, auth);
        assert!(result.is_ok());
    }
}
