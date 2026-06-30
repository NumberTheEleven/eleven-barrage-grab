//! URL 解析器（R-001）
//!
//! 从用户输入 URL 提取 `web_rid`，支持以下抖音直播间长链：
//!
//! # 支持的格式
//!
//! - `https://live.douyin.com/664637748606`
//! - `live.douyin.com/664637748606`（无 scheme，自动补 `https://`）
//! - `live.douyin.com/xxx?foo=bar`（含 query，query 忽略）
//! - `live.douyin.com/xxx#section`（含 fragment，fragment 忽略）
//! - `https://www.douyin.com/root/live/49844349625`
//! - `www.douyin.com/root/live/49844349625?anchor_id=...`
//!
//! # 拒绝的格式
//!
//! - `v.douyin.com/abc`（其他域名 → `UrlFormatNotSupported`）
//! - `""`（空字符串 → `EmptyUrl`）
//! - `https://example.com/xxx`（非抖音域名 → `UrlFormatNotSupported`）

use crate::error::SignatureError;

/// 解析后的 web_rid（web 直播间标识）
pub type WebRid = String;

/// 抖音直播间的 host（白名单）
const ALLOWED_HOSTS: &[&str] = &["live.douyin.com", "www.douyin.com"];

/// 从用户输入 URL 提取 web_rid
///
/// # 参数
///
/// - `input`: 用户输入的 URL 字符串（可带或不带 scheme）
///
/// # 返回
///
/// - `Ok(WebRid)`: 成功提取的 web_rid
/// - `Err(SignatureError)`: 解析失败（带结构化错误）
///
/// # 示例
///
/// ```ignore
/// let web_rid = parse("https://live.douyin.com/664637748606")?;
/// assert_eq!(web_rid, "664637748606");
/// ```
pub fn parse(input: &str) -> Result<WebRid, SignatureError> {
    // 1. 空字符串检查
    if input.trim().is_empty() {
        return Err(SignatureError::EmptyUrl);
    }

    // 2. 尝试解析 URL（无 scheme 时补 https://）
    let normalized = ensure_scheme(input.trim());
    let url = url::Url::parse(&normalized).map_err(|_| SignatureError::UrlFormatNotSupported {
        url: input.to_string(),
    })?;

    // 3. 域名白名单
    let host = url
        .host_str()
        .ok_or_else(|| SignatureError::UrlFormatNotSupported {
            url: input.to_string(),
        })?;
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(SignatureError::UrlFormatNotSupported {
            url: input.to_string(),
        });
    }

    // 4. 路径提取 web_rid
    let path = url.path();
    let web_rid = if host == "www.douyin.com" {
        extract_www_root_live_web_rid(path, input)?
    } else {
        extract_first_path_segment(path, input)?
    };

    Ok(web_rid.to_string())
}

fn extract_first_path_segment<'a>(path: &'a str, input: &str) -> Result<&'a str, SignatureError> {
    path.trim_start_matches('/')
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SignatureError::UrlFormatNotSupported {
            url: input.to_string(),
        })
}

fn extract_www_root_live_web_rid<'a>(path: &'a str, input: &str) -> Result<&'a str, SignatureError> {
    let mut segments = path.trim_start_matches('/').split('/');
    match (segments.next(), segments.next(), segments.next()) {
        (Some("root"), Some("live"), Some(web_rid)) if !web_rid.is_empty() => Ok(web_rid),
        _ => Err(SignatureError::UrlFormatNotSupported {
            url: input.to_string(),
        }),
    }
}

/// 确保 URL 有 scheme（无则补 `https://`）
fn ensure_scheme(input: &str) -> String {
    if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{}", input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== 成功路径 =====

    #[test]
    fn parse_full_url_with_https() {
        let result = parse("https://live.douyin.com/664637748606");
        assert_eq!(result.unwrap(), "664637748606");
    }

    #[test]
    fn parse_url_without_scheme() {
        let result = parse("live.douyin.com/664637748606");
        assert_eq!(result.unwrap(), "664637748606");
    }

    #[test]
    fn parse_url_with_query() {
        let result = parse("https://live.douyin.com/xxx?foo=bar&baz=qux");
        assert_eq!(result.unwrap(), "xxx");
    }

    #[test]
    fn parse_url_with_fragment() {
        let result = parse("https://live.douyin.com/xxx#section");
        assert_eq!(result.unwrap(), "xxx");
    }

    #[test]
    fn parse_url_with_query_and_fragment() {
        let result = parse("https://live.douyin.com/abc?x=1#frag");
        assert_eq!(result.unwrap(), "abc");
    }

    #[test]
    fn parse_url_with_http_scheme() {
        let result = parse("http://live.douyin.com/664637748606");
        assert_eq!(result.unwrap(), "664637748606");
    }

    #[test]
    fn parse_long_web_rid() {
        let result = parse("https://live.douyin.com/741891423654123456789");
        assert_eq!(result.unwrap(), "741891423654123456789");
    }

    // ===== 失败路径 =====

    #[test]
    fn reject_empty_string() {
        let result = parse("");
        assert!(matches!(result, Err(SignatureError::EmptyUrl)));
    }

    #[test]
    fn reject_whitespace_only() {
        let result = parse("   ");
        assert!(matches!(result, Err(SignatureError::EmptyUrl)));
    }

    #[test]
    fn reject_v_douyin_com() {
        let result = parse("https://v.douyin.com/abc123");
        assert!(matches!(
            result,
            Err(SignatureError::UrlFormatNotSupported { .. })
        ));
    }

    #[test]
    fn reject_other_domain() {
        let result = parse("https://example.com/xxx");
        assert!(matches!(
            result,
            Err(SignatureError::UrlFormatNotSupported { .. })
        ));
    }

    #[test]
    fn parse_www_douyin_com_root_live() {
        let result = parse("https://www.douyin.com/root/live/49844349625");
        assert_eq!(result.unwrap(), "49844349625");
    }

    #[test]
    fn parse_www_douyin_com_root_live_with_query() {
        let result = parse(
            "https://www.douyin.com/root/live/49844349625?anchor_id=66730156188&use_new_preview=true",
        );
        assert_eq!(result.unwrap(), "49844349625");
    }

    #[test]
    fn parse_www_douyin_com_root_live_without_scheme() {
        let result = parse("www.douyin.com/root/live/49844349625");
        assert_eq!(result.unwrap(), "49844349625");
    }

    #[test]
    fn reject_www_douyin_com_other_path() {
        let result = parse("https://www.douyin.com/xxx");
        assert!(matches!(
            result,
            Err(SignatureError::UrlFormatNotSupported { .. })
        ));
    }

    #[test]
    fn reject_www_douyin_com_missing_id() {
        let result = parse("https://www.douyin.com/root/live/");
        assert!(matches!(
            result,
            Err(SignatureError::UrlFormatNotSupported { .. })
        ));
    }

    #[test]
    fn reject_malformed_url() {
        let result = parse("not a url at all");
        assert!(matches!(
            result,
            Err(SignatureError::UrlFormatNotSupported { .. })
        ));
    }

    #[test]
    fn reject_path_only() {
        let result = parse("https://live.douyin.com/");
        assert!(matches!(
            result,
            Err(SignatureError::UrlFormatNotSupported { .. })
        ));
    }

    // ===== 错误属性验证 =====

    #[test]
    fn url_format_error_not_retryable() {
        let result = parse("https://v.douyin.com/abc");
        match result {
            Err(e) => assert!(!e.retryable()),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn empty_url_error_not_retryable() {
        let result = parse("");
        match result {
            Err(e) => assert!(!e.retryable()),
            Ok(_) => panic!("expected error"),
        }
    }

    // ===== ensure_scheme 内部函数测试 =====

    #[test]
    fn ensure_scheme_adds_https() {
        assert_eq!(
            ensure_scheme("live.douyin.com/xxx"),
            "https://live.douyin.com/xxx"
        );
    }

    #[test]
    fn ensure_scheme_keeps_existing() {
        assert_eq!(
            ensure_scheme("https://live.douyin.com/xxx"),
            "https://live.douyin.com/xxx"
        );
        assert_eq!(
            ensure_scheme("http://live.douyin.com/xxx"),
            "http://live.douyin.com/xxx"
        );
    }
}
