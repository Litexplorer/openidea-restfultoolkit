//! 配置文件解析（US-E1-05）：从 `application*.properties` / `application*.yml` 的文本里
//! 取 `server.port`、context-path 与激活 profile。纯函数：不读文件，不展开占位符表达式。

/// 参数：`value` 为原始值，可能带引号或 `${NAME:默认值}` 占位符。
/// 核心流程：去空白 → 去成对引号 → 占位符只取冒号后的默认值；无默认值视为不可解析（None）。
fn clean(value: &str) -> Option<String> {
    let v = value.trim().trim_matches(|c| c == '"' || c == '\'');
    if v.is_empty() {
        return None;
    }
    let Some(inner) = v.strip_prefix("${").and_then(|rest| rest.strip_suffix('}')) else {
        return Some(v.to_string());
    };
    inner.split_once(':').map(|(_, default)| default.trim().to_string()).filter(|d| !d.is_empty())
}

/// 参数：`line` 为一行原文。
/// 核心流程：找到行首或以空白开头的 `#`，其后全部视为注释丢掉。
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &line[..i];
        }
    }
    line
}

/// 参数：`body` 为去掉注释与首尾空白的单行文本。
/// 核心流程：优先按第一个 `=` 拆键值，否则按第一个 `:` 拆（properties 与 yml 共用）。
fn split_kv(body: &str) -> Option<(&str, &str)> {
    match body.find('=') {
        Some(i) => Some((body[..i].trim(), body[i + 1..].trim())),
        None => body.find(':').map(|i| (body[..i].trim(), body[i + 1..].trim())),
    }
}

/// 参数：`text` 为单个配置文件的全文。
/// 核心流程：逐行去注释 → 按缩进栈把 yml 的嵌套键展平成 `server.servlet.context-path`
/// 这样的点号键 → 收集全部叶子键值对（同键多次出现时保持出现顺序）。
fn pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for raw in text.lines() {
        let line = strip_comment(raw);
        let body = line.trim();
        if body.is_empty() || body == "---" || body.starts_with('-') {
            if body == "---" {
                stack.clear();
            }
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some((key, value)) = split_kv(body) else { continue };
        if key.is_empty() {
            continue;
        }
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let mut full = String::new();
        for (_, parent) in &stack {
            full.push_str(parent);
            full.push('.');
        }
        full.push_str(key);
        if value.is_empty() {
            stack.push((indent, key.to_string()));
        } else {
            out.push((full, value.to_string()));
        }
    }
    out
}

/// 参数：`text` 为配置文件全文（properties 或 yml）。
/// 核心流程：取最后一个 `server.port`，去引号/占位符后按 u16 解析；解析不了返回 None。
pub fn read_server_port(text: &str) -> Option<u16> {
    let pairs = pairs(text);
    let (_, value) = pairs.iter().rev().find(|(key, _)| key == "server.port")?;
    clean(value)?.parse().ok()
}

/// 参数：`text` 为配置文件全文（properties 或 yml）。
/// 核心流程：优先新键 `server.servlet.context-path`，缺失时回落到旧键 `server.context-path`；
/// 值去引号/占位符后原样返回（斜杠归一化交给 `url::normalize_path`）。
pub fn read_context_path(text: &str) -> Option<String> {
    let pairs = pairs(text);
    let last = |want: &str| {
        pairs.iter().rev().find(|(key, _)| key == want).and_then(|(_, value)| clean(value))
    };
    last("server.servlet.context-path").or_else(|| last("server.context-path"))
}

/// 参数：`text` 为配置文件全文（properties 或 yml）。
/// 核心流程：取 `spring.profiles.active`，逗号列表只认第一个非空 profile。
pub fn read_active_profile(text: &str) -> Option<String> {
    let pairs = pairs(text);
    let (_, value) = pairs.iter().rev().find(|(key, _)| key == "spring.profiles.active")?;
    let value = clean(value)?;
    value.split(',').map(str::trim).find(|p| !p.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_properties() {
        let text = "server.port=8081\nserver.servlet.context-path=/gateway\nspring.profiles.active=dev\n";
        assert_eq!(read_server_port(text), Some(8081));
        assert_eq!(read_context_path(text).as_deref(), Some("/gateway"));
        assert_eq!(read_active_profile(text).as_deref(), Some("dev"));
    }

    #[test]
    fn reads_nested_yml() {
        let text = "server:\n  port: 8081\n  servlet:\n    context-path: /gateway\nspring:\n  profiles:\n    active: dev\n";
        assert_eq!(read_server_port(text), Some(8081));
        assert_eq!(read_context_path(text).as_deref(), Some("/gateway"));
        assert_eq!(read_active_profile(text).as_deref(), Some("dev"));
    }

    #[test]
    fn reads_flat_yml_keys() {
        let text = "server.port: 9090\nserver.context-path: /old\n";
        assert_eq!(read_server_port(text), Some(9090));
        assert_eq!(read_context_path(text).as_deref(), Some("/old"), "旧键仍需识别");
    }

    #[test]
    fn missing_config_is_none() {
        let text = "# 什么都没有\nspring:\n  application:\n    name: demo\n";
        assert_eq!(read_server_port(text), None);
        assert_eq!(read_context_path(text), None);
        assert_eq!(read_active_profile(text), None);
    }

    #[test]
    fn quoted_values_lose_their_quotes() {
        let text = "server.port: \"8088\"\nserver.servlet.context-path: '/api'\n";
        assert_eq!(read_server_port(text), Some(8088));
        assert_eq!(read_context_path(text).as_deref(), Some("/api"));
    }

    #[test]
    fn placeholders_take_the_default_only() {
        assert_eq!(read_server_port("server.port=${PORT:9091}"), Some(9091));
        assert_eq!(read_server_port("server.port=${PORT}"), None, "不解析占位符原文");
    }

    #[test]
    fn later_lines_and_profile_lists_win() {
        let text = "server.port=8081\nserver.port=${PORT:8082}\nspring.profiles.active=dev,test\n";
        assert_eq!(read_server_port(text), Some(8082));
        assert_eq!(read_active_profile(text).as_deref(), Some("dev"));
    }
}
