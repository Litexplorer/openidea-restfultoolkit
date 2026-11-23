//! 响应文本处理（US-E4-03）：JSON 美化与按上限截断。

/// `Pane::Text` 与剪贴板共同的 1 MiB 上限（宿主会静默截断）。
pub const TEXT_LIMIT: usize = 1 << 20;

/// 超过它就应改写文件而不是剪贴板（US-E5-02：900 KiB）。
pub const CLIPBOARD_SAFE_LIMIT: usize = 900 * 1024;

/// 参数：`text` 为响应的原始文本。
/// 核心流程：`serde_json` 解析成功就输出两空格缩进版；不是 JSON（HTML、纯文本）返回 None。
pub fn pretty_json(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// 参数：`text` 为待截断文本，`max` 为字节上限。
/// 核心流程：没超上限原样返回；超了在 UTF-8 字符边界处截断（不切坏中文/emoji）并附一行「已截断」提示。
pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（已截断）", &text[..end])
}

/// 参数：`text` 为要复制的文本。
/// 核心流程：≤900 KiB 原样返回；超过则给一句可读提示（写文件而不是交给 1 MiB 静默截断），
/// 判长在插件侧完成，不依赖宿主。
pub fn for_clipboard(text: &str) -> Result<String, String> {
    if text.len() <= CLIPBOARD_SAFE_LIMIT {
        return Ok(text.to_string());
    }
    Err(format!(
        "内容过大（{} KiB > {} KiB）：请写入 fs.plugin_dir，或缩小范围后再复制",
        text.len() / 1024,
        CLIPBOARD_SAFE_LIMIT / 1024
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretifies_json_and_rejects_other_text() {
        assert_eq!(pretty_json(r#"{"a":1}"#).as_deref(), Some("{\n  \"a\": 1\n}"));
        assert_eq!(pretty_json("<html></html>"), None);
    }

    #[test]
    fn short_text_is_returned_unchanged() {
        assert_eq!(truncate("hello", 32), "hello");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn long_text_is_cut_on_a_char_boundary() {
        let cut = truncate("abcdef", 4);
        assert!(cut.starts_with("abcd"), "{cut}");
        assert!(cut.contains("已截断"), "{cut}");
        // 「中文」6 字节，上限 4 只能放下一个汉字。
        let cjk = truncate("中文", 4);
        assert!(cjk.starts_with('中'), "{cjk}");
        assert!(cjk.contains("已截断"), "{cjk}");
    }

    #[test]
    fn the_clipboard_limit_is_900_kib_with_a_readable_message() {
        assert_eq!(CLIPBOARD_SAFE_LIMIT, 900 * 1024);
        assert!(for_clipboard("short").is_ok());
        let big = "x".repeat(CLIPBOARD_SAFE_LIMIT + 1);
        let message = for_clipboard(&big).unwrap_err();
        assert!(message.contains("内容过大"), "{message}");
        assert!(message.contains("900"), "{message}");
        assert!(for_clipboard(&"x".repeat(CLIPBOARD_SAFE_LIMIT)).is_ok(), "正好 900 KiB 仍可复制");
    }

    #[test]
    fn the_text_limit_is_one_mebibyte() {
        assert_eq!(TEXT_LIMIT, 1024 * 1024);
        let big = "x".repeat(TEXT_LIMIT + 10);
        let cut = truncate(&big, TEXT_LIMIT);
        assert!(cut.len() < TEXT_LIMIT + 64, "截断后必须落在上限附近");
        assert!(cut.contains("已截断"));
    }
}
