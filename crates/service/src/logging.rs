//! tracing 日志初始化
//!
//! 支持两种格式：
//! - **JSON**（生产）：结构化日志，便于日志聚合
//! - **Pretty**（开发）：人类可读，便于调试

use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::config::LoggingConfig;

/// 初始化 tracing subscriber
pub fn init(config: &LoggingConfig) -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(env_filter);

    if config.json_format {
        registry
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(false)
                    .with_span_list(false),
            )
            .try_init()
            .map_err(|e| anyhow::anyhow!("failed to init tracing subscriber: {}", e))?;
    } else {
        registry
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(false)
                    .with_line_number(false),
            )
            .try_init()
            .map_err(|e| anyhow::anyhow!("failed to init tracing subscriber: {}", e))?;
    }

    tracing::info!(
        level = %config.level,
        json_format = config.json_format,
        "tracing initialized"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_init_with_default_config() {
        let cfg = LoggingConfig::default();
        // 注意：tracing subscriber 全局唯一，重复初始化会失败但不影响功能
        let _ = init(&cfg);
    }
}
