//! URL 拼装与路径归一化（US-E3-01 复制完整 URL）。
//!
//! 本迭代只做「拼装 + 归一化」，不处理 query 参数，也不替换 `{var}` 占位符。

/// 归一化 URL 路径。
///
/// 参数：`p` 为任意待归一化的路径片段，可含 `{var}` 占位符，允许前后空白。
/// 核心流程：去首尾空白 → 逐字符合并连续 `/` → 补前导 `/` → 去尾部 `/`
/// （根路径除外）；空串归一为 `/`，`{var}` 等内容原样保留。
pub fn normalize_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 1);
    for ch in p.trim().chars() {
        // 已有一个 `/` 时跳过后续的 `/`，即合并连续斜杠。
        if ch == '/' && out.ends_with('/') {
            continue;
        }
        out.push(ch);
    }
    if !out.starts_with('/') {
        out.insert(0, '/');
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

/// 拼装完整 URL。
///
/// 参数：`base_url` 为协议与主机（可带端口，如 `http://localhost:8080`）；
/// `base_path` 为上下文路径 + 类级映射；`method_path` 为方法级映射。
/// 核心流程：先去 `base_url` 末尾 `/`（去掉后为空则只返回相对路径）→
/// 依序拼接 `base_path` 与 `method_path` → 交给 `normalize_path` 统一归一化，
/// 因此片段之间的斜杠不会重复。
pub fn join_url(base_url: &str, base_path: &str, method_path: &str) -> String {
    let mut base = base_url.trim();
    // 保留 `http://` 这类协议后缀，只去主机后的多余斜杠。
    while base.ends_with('/') && !base.ends_with("://") {
        base = &base[..base.len() - 1];
    }
    let path = normalize_path(&format!("{}{}", base_path.trim(), method_path.trim()));
    if base.is_empty() {
        return path;
    }
    format!("{}{}", base, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_blank_and_spaces() {
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("   "), "/");
        assert_eq!(normalize_path("  /api  "), "/api");
    }

    #[test]
    fn normalize_collapses_double_slashes() {
        assert_eq!(normalize_path("//api///v1//users"), "/api/v1/users");
        assert_eq!(join_url("http://localhost:8080", "/api/", "//users"), "http://localhost:8080/api/users");
    }

    #[test]
    fn normalize_strips_trailing_slash_but_keeps_root() {
        assert_eq!(normalize_path("/api/"), "/api");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("///"), "/");
        assert_eq!(normalize_path("api/"), "/api");
    }

    #[test]
    fn normalize_keeps_placeholder() {
        assert_eq!(normalize_path("/users/{id}/"), "/users/{id}");
        assert_eq!(normalize_path("{id}"), "/{id}");
    }

    #[test]
    fn join_full_url_with_port_and_placeholder() {
        assert_eq!(
            join_url("http://localhost:8080", "/api", "/v1/users/{id}"),
            "http://localhost:8080/api/v1/users/{id}"
        );
    }

    #[test]
    fn join_trailing_slash_in_base_url() {
        assert_eq!(join_url("http://localhost:8080/", "/api/", "/users/"), "http://localhost:8080/api/users");
        assert_eq!(join_url("http://localhost:8080/", "", ""), "http://localhost:8080/");
    }

    #[test]
    fn join_empty_base_returns_relative_path() {
        assert_eq!(join_url("", "/api", "/users"), "/api/users");
        assert_eq!(join_url("   ", "/api", ""), "/api");
    }
}
