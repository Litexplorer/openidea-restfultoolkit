//! 请求 URL 的拼装细节（US-E4-02）：`{var}` 替换与 query 拼接都是纯函数。

/// 参数：`url` 为待替换的 URL，`kv` 为变量表（键是占位符名，不含花括号）。
/// 核心流程：从左到右找 `{name}`；键在表里就换成值，不在表里原样保留（含 `{id:[0-9]+}` 这类表达式）。
pub fn apply_path_vars(url: &str, kv: &[(String, String)]) -> String {
    let mut out = String::with_capacity(url.len());
    let mut rest = url;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|i| i + open) else { break };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..close];
        match kv.iter().find(|(key, _)| key == name) {
            Some((_, value)) => out.push_str(&encode(value)),
            None => out.push_str(&rest[open..=close]),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// 参数：`url` 为已经带好路径的 URL，`query` 为 `k=v&k=v` 形态的查询串。
/// 核心流程：查询串为空就原样返回；已以 `?`/`&` 结尾直接接上，否则看有没有 `?`
/// 决定用 `&` 还是 `?`，不产生 `??` 或空查询。
pub fn append_query(url: &str, query: &str) -> String {
    let query = query.trim().trim_start_matches(['?', '&']);
    if query.is_empty() {
        return url.to_string();
    }
    let sep = if url.ends_with('?') || url.ends_with('&') {
        ""
    } else if url.contains('?') {
        "&"
    } else {
        "?"
    };
    format!("{url}{sep}{query}")
}

/// 参数：`value` 为要放进 URL 的原始值。
/// 核心流程：只保留 RFC 3986 非保留字符（字母、数字、`-_.~`），其余按 UTF-8 字节写成 `%XX`；
/// 中文、空格与 `&` 都会被编码，路径占位符本身在替换前不动。
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// 参数：`url` 为原始 URL，`kv` 为已提供的变量表。
/// 核心流程：找出 URL 里出现但变量表没有的占位符名（去重、保持出现顺序）。
pub fn missing_vars(url: &str, kv: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = url;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|i| i + open) else { break };
        let name = &rest[open + 1..close];
        if !kv.iter().any(|(key, _)| key == name) && !out.iter().any(|seen| seen == name) {
            out.push(name.to_string());
        }
        rest = &rest[close + 1..];
    }
    out
}

/// 参数：`input` 为命令输入，可为 `METHOD URL` 或只有 `URL`。
/// 核心流程：第一个词是已知 HTTP 方法就当方法（统一大写），其余全是 URL；否则整串是 URL、方法默认 GET。
pub fn parse_request_line(input: &str) -> (String, String) {
    let input = input.trim();
    if let Some((first, rest)) = input.split_once(char::is_whitespace) {
        if is_method(first) && !rest.trim().is_empty() {
            return (first.to_ascii_uppercase(), rest.trim().to_string());
        }
    }
    ("GET".to_string(), input.to_string())
}

/// 参数：`input` 为命令输入 `METHOD URL [k=v ...]`（方法可省略）。
/// 核心流程：切出方法与 URL，其余 `k=v` 记成变量表（路径变量与 query 都从这里来）；
/// 不是 `k=v` 的尾巴忽略，不 panic。
pub fn parse_request_input(input: &str) -> (String, String, Vec<(String, String)>) {
    let mut words = input.split_whitespace();
    let first = words.next().unwrap_or_default();
    let (method, url) = if is_method(first) {
        (first.to_ascii_uppercase(), words.next().unwrap_or_default().to_string())
    } else {
        ("GET".to_string(), first.to_string())
    };
    let pairs = words
        .filter_map(|word| word.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    (method, url, pairs)
}

/// 参数：`word` 为候选方法名。
/// 核心流程：大小写不敏感地比对七个常见方法，其余视为 URL 的一部分。
fn is_method(word: &str) -> bool {
    matches!(
        word.to_ascii_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn path_vars_are_replaced() {
        assert_eq!(
            apply_path_vars("http://localhost:8080/api/users/{id}", &kv(&[("id", "42")])),
            "http://localhost:8080/api/users/42"
        );
    }

    #[test]
    fn missing_path_vars_stay_as_written() {
        assert_eq!(
            apply_path_vars("http://localhost/api/users/{id}/orders/{orderId}", &kv(&[("id", "1")])),
            "http://localhost/api/users/1/orders/{orderId}"
        );
    }

    #[test]
    fn regex_placeholders_are_not_touched() {
        let url = "http://localhost/api/items/{id:[0-9]+}";
        assert_eq!(apply_path_vars(url, &kv(&[("id", "7")])), url);
    }

    #[test]
    fn query_is_appended_with_the_right_separator() {
        assert_eq!(append_query("http://localhost/api/users", "name=alice&age=0"), "http://localhost/api/users?name=alice&age=0");
        assert_eq!(
            append_query("http://localhost/api/users?debug=1", "name=alice"),
            "http://localhost/api/users?debug=1&name=alice"
        );
    }

    #[test]
    fn empty_query_leaves_the_url_alone() {
        assert_eq!(append_query("http://localhost/api/users?", ""), "http://localhost/api/users?");
        assert_eq!(append_query("http://localhost/api/users?", "?&"), "http://localhost/api/users?");
    }

    #[test]
    fn values_are_percent_encoded_after_replacement() {
        let pairs = vec![("id".to_string(), "a b中文&x".to_string())];
        assert_eq!(apply_path_vars("http://h/api/{id}", &pairs), "http://h/api/a%20b%E4%B8%AD%E6%96%87%26x");
        assert_eq!(encode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn missing_vars_are_reported_by_name() {
        let pairs = vec![("id".to_string(), "1".to_string())];
        assert_eq!(missing_vars("http://h/api/{id}/orders/{orderId}", &pairs), vec!["orderId".to_string()]);
        assert!(missing_vars("http://h/api/{id}", &pairs).is_empty());
    }

    #[test]
    fn request_input_collects_variables() {
        let (method, url, pairs) = parse_request_input("POST http://localhost:8080/api/users/{id} id=42 name=alice");
        assert_eq!(method, "POST");
        assert_eq!(url, "http://localhost:8080/api/users/{id}");
        assert_eq!(pairs, vec![("id".to_string(), "42".to_string()), ("name".to_string(), "alice".to_string())]);
        assert_eq!(apply_path_vars(&url, &pairs), "http://localhost:8080/api/users/42");
    }

    #[test]
    fn request_lines_may_or_may_not_name_the_method() {
        assert_eq!(
            parse_request_line("POST http://localhost:8080/api/users"),
            ("POST".to_string(), "http://localhost:8080/api/users".to_string())
        );
        assert_eq!(
            parse_request_line("http://localhost:8080/api/users"),
            ("GET".to_string(), "http://localhost:8080/api/users".to_string())
        );
    }
}
