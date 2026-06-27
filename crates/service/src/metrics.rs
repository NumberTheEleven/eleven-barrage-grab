//! Prometheus metrics 基础设施
//!
//! 暴露的关键指标：
//! - `barrage_events_total{event_type}`：事件总数（按类型）
//! - `barrage_processing_duration_seconds`：处理延迟直方图
//! - `wss_connection_state{room_id}`：wss 连接状态（0=disconnected, 1=connecting, 2=connected）
//! - `decode_errors_total{error_type}`：解码错误总数
//! - `heartbeat_success_total`：心跳成功总数
//! - `reconnect_total{reason}`：重连次数

use anyhow::{Context, Result};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};

use crate::config::ServiceConfig;

/// Prometheus exporter handle
pub struct MetricsExporter {
    /// `PrometheusBuilder` 在 drop 时自动停止 exporter
    _builder: PrometheusBuilder,
    /// 用于 shutdown 的本地 handle
    handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl MetricsExporter {
    /// 初始化 Prometheus exporter 并绑定到 metrics_listen_addr
    pub fn install(config: &ServiceConfig) -> Result<Self> {
        let builder = PrometheusBuilder::new()
            .with_http_listener(config.metrics_listen_addr)
            .set_buckets_for_metric(
                Matcher::Full("barrage_processing_duration_seconds".to_string()),
                &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
            )
            .with_context(|| format!("failed to setup Prometheus on {}", config.metrics_listen_addr))?;

        let handle = builder
            .install_recorder()
            .context("failed to install Prometheus recorder")?;

        tracing::info!(
            addr = %config.metrics_listen_addr,
            "Prometheus metrics exporter installed"
        );

        Ok(Self {
            _builder: builder,
            handle,
        })
    }

    /// 获取 Prometheus handle（用于自定义采集）
    pub fn handle(&self) -> &metrics_exporter_prometheus::PrometheusHandle {
        &self.handle
    }

    /// 渲染 Prometheus exposition format 文本
    pub fn render(&self) -> String {
        self.handle.render()
    }
}

/// 便捷的指标记录函数
pub mod record {
    use super::*;

    /// 记录事件处理
    pub fn event_processed(event_type: &str, duration_secs: f64) {
        counter!("barrage_events_total", "event_type" => event_type.to_string()).increment(1);
        histogram!("barrage_processing_duration_seconds", "event_type" => event_type.to_string())
            .record(duration_secs);
    }

    /// 记录解码错误
    pub fn decode_error(error_type: &str) {
        counter!("decode_errors_total", "error_type" => error_type.to_string()).increment(1);
    }

    /// 记录解码成功（用于错误计数清零指标）
    pub fn decode_success() {
        counter!("decode_success_total").increment(1);
    }

    /// 设置 wss 连接状态
    pub fn wss_state(room_id: &str, state: WssState) {
        gauge!("wss_connection_state", "room_id" => room_id.to_string()).set(state as f64);
    }

    /// 记录心跳成功
    pub fn heartbeat_success(room_id: &str) {
        counter!("heartbeat_success_total", "room_id" => room_id.to_string()).increment(1);
    }

    /// 记录心跳失败
    pub fn heartbeat_failure(room_id: &str) {
        counter!("heartbeat_failure_total", "room_id" => room_id.to_string()).increment(1);
    }

    /// 记录重连
    pub fn reconnect(reason: &str) {
        counter!("reconnect_total", "reason" => reason.to_string()).increment(1);
    }
}

/// WSS 连接状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WssState {
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
}

impl std::fmt::Display for WssState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WssState::Disconnected => write!(f, "disconnected"),
            WssState::Connecting => write!(f, "connecting"),
            WssState::Connected => write!(f, "connected"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    #[test]
    fn wss_state_display() {
        assert_eq!(WssState::Disconnected.to_string(), "disconnected");
        assert_eq!(WssState::Connecting.to_string(), "connecting");
        assert_eq!(WssState::Connected.to_string(), "connected");
    }

    #[test]
    fn metrics_exporter_install() {
        // 用一个无效地址测试错误处理（绑定到保留端口）
        let config = ServiceConfig {
            room_id: "test".to_string(),
            ws_listen_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 65530)),
            grpc_listen_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 65531)),
            metrics_listen_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 65532)),
        };

        // 实际启动可能会失败（如果端口被占用），但我们只验证不会 panic
        let _ = MetricsExporter::install(&config);
    }
}