//! WebSocket 路径解析工具
//!
//! 解析 HTTP upgrade 请求的 URI 路径，识别 `/rooms/<id>` 形式的房间订阅端点。
//!
//! # 约定
//!
//! - `/rooms/<id>`：合法，提取 `id`
//! - `/`、`/rooms`、`/rooms/`：非法，返回 `None`
//! - 解码后的 `id` 必须非空
//!
//! 该模块故意做成纯函数，便于单测。

/// 解析 `/rooms/<id>` 形式的路径，返回房间 ID。
///
/// # 返回
///
/// - `Some(id)`：合法路径，提取出 `id`
/// - `None`：路径不匹配预期形式
pub fn parse_room_path(path: &str) -> Option<&str> {
    // 去掉开头多余的 `/`
    let trimmed = path.trim_start_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    if parts[0] != "rooms" {
        return None;
    }
    let id = parts[1];
    if id.is_empty() {
        return None;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legal_room_path() {
        assert_eq!(parse_room_path("/rooms/abc"), Some("abc"));
    }

    #[test]
    fn parses_room_path_with_numeric_id() {
        assert_eq!(parse_room_path("/rooms/741891423654"), Some("741891423654"));
    }

    #[test]
    fn rejects_root() {
        assert_eq!(parse_room_path("/"), None);
    }

    #[test]
    fn rejects_only_rooms() {
        assert_eq!(parse_room_path("/rooms"), None);
    }

    #[test]
    fn rejects_trailing_slash() {
        assert_eq!(parse_room_path("/rooms/"), None);
    }

    #[test]
    fn rejects_extra_segments() {
        assert_eq!(parse_room_path("/rooms/abc/extra"), None);
    }

    #[test]
    fn rejects_wrong_prefix() {
        assert_eq!(parse_room_path("/foo/abc"), None);
    }

    #[test]
    fn rejects_without_leading_slash() {
        // 客户端可能省略前导 `/`
        assert_eq!(parse_room_path("rooms/abc"), Some("abc"));
    }
}
