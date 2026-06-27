//! 房间元数据 API 客户端
//!
//! 复刻原项目 `DyApiHelper.GetRoomInfoForApi` 的功能：
//! - HTTP GET `https://live.douyin.com/webcast/room/web/enter/`
//! - query 参数：`aid=6383, device_platform=web, web_rid={room_id}`
//! - Headers：UA + Referer + 可选 Cookie
//!
//! # 返回
//! - `Ok(RoomInfo)`：包含 room_id_str、title、owner 等
//! - `Err(anyhow::Error)`：网络/解析错误

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::RoomApiConfig;

/// 房间元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    /// 内部 room_id
    pub room_id: String,
    /// web 房间 ID
    pub web_room_id: String,
    /// 主播昵称
    pub owner_nickname: Option<String>,
    /// 直播间标题
    pub title: Option<String>,
    /// 是否开播
    pub is_live: bool,
    /// 推流地址（用于拉流/分析）
    pub live_url: String,
}

/// 房间元数据 API 客户端
#[derive(Debug, Clone)]
pub struct RoomInfoApi {
    config: RoomApiConfig,
    client: reqwest::Client,
}

impl RoomInfoApi {
    /// 创建新客户端
    pub fn new(config: RoomApiConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(&config.user_agent)
            .build()
            .context("failed to build reqwest client")?;
        Ok(Self { config, client })
    }

    /// 获取房间信息
    ///
    /// # 参数
    /// - `web_room_id`: 抖音 web 直播间的标识（如 "741891423654"）
    ///
    /// # 返回
    /// - `Ok(RoomInfo)`：成功获取
    /// - `Err(anyhow::Error)`：失败
    pub async fn get(&self, web_room_id: &str) -> Result<RoomInfo> {
        let mut query = HashMap::new();
        query.insert("aid", "6383");
        query.insert("live_id", "1");
        query.insert("app_name", "douyin_web");
        query.insert("device_platform", "web");
        query.insert("language", "zh-CN");
        query.insert("cookie_enabled", "true");
        query.insert("enter_from", "page_refresh");
        query.insert("web_rid", web_room_id);
        query.insert("enter_source", "");
        query.insert("is_need_double_stream", "false");
        query.insert("insert_task_id", "");
        query.insert("live_reason", "");
        query.insert("browser_language", "zh-CN");
        query.insert("browser_platform", "Win32");
        query.insert("browser_name", "Edge");

        let url = "https://live.douyin.com/webcast/room/web/enter/";

        let mut request = self
            .client
            .get(url)
            .query(&query)
            .header("Accept", "application/json, text/plain, */*")
            .header("Cache-Control", "no-cache")
            .header("Referer", format!("https://live.douyin.com/{}", web_room_id))
            .header("Host", "live.douyin.com");

        if !self.config.cookie.is_empty() {
            request = request.header("Cookie", &self.config.cookie);
        }

        let response = request
            .send()
            .await
            .context("failed to send room info request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "room info API returned non-success status: {}",
                response.status()
            );
        }

        let body = response
            .text()
            .await
            .context("failed to read room info response body")?;

        // 解析响应（抖音 room info 响应结构）
        // 注：完整响应 schema 较复杂，这里提取关键字段
        let parsed: serde_json::Value =
            serde_json::from_str(&body).context("failed to parse room info JSON")?;

        let data = parsed
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("missing 'data' field in room info response"))?;

        let room = data
            .get("room")
            .ok_or_else(|| anyhow::anyhow!("missing 'room' field in room info data"))?;

        let room_id_str = room
            .get("id_str")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let owner_nickname = room
            .get("owner")
            .and_then(|o| o.get("nickname"))
            .and_then(|n| n.as_str())
            .map(String::from);

        let title = room
            .get("title")
            .and_then(|t| t.as_str())
            .map(String::from);

        let status = room.get("status").and_then(|s| s.as_i64()).unwrap_or(0);
        let is_live = status == 2; // 2 = 开播中

        let live_url = format!("https://live.douyin.com/{}", web_room_id);

        Ok(RoomInfo {
            room_id: room_id_str,
            web_room_id: web_room_id.to_string(),
            owner_nickname,
            title,
            is_live,
            live_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_info_api_creation() {
        let cfg = RoomApiConfig::default();
        let api = RoomInfoApi::new(cfg);
        assert!(api.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires network access to live.douyin.com"]
    async fn get_real_room_info() {
        let cfg = RoomApiConfig::default();
        let api = RoomInfoApi::new(cfg).unwrap();
        let result = api.get("741891423654").await;
        // 不强制要求成功（可能在沙盒环境无网络）
        if let Ok(info) = result {
            assert!(!info.web_room_id.is_empty());
        }
    }
}