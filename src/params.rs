//! 方法参数列表 → query 骨架（US-E3-02）：`name=demoData&age=1`。
//!
//! 跳过 `@RequestBody` 参数；基础类型给示例值；对象参数展开成它的基础字段
//! （`serialVersionUID` 等静态常量不出现）；键名取 `@RequestParam` 的名字与默认值。

use crate::http::encode;
use crate::json_gen::fields_of;

/// 一个方法参数：变量名、类型原文、是否 `@RequestBody`、query 键名与注解里的默认值。
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: String,
    pub body: bool,
    pub key: String,
    pub default: Option<String>,
}

/// 参数：`params` 为方法括号内（或整段含括号）的参数列表原文。
/// 核心流程：按顶层逗号切分（忽略泛型尖括号里的逗号）→ 每段取注解标记与
/// `@RequestParam` 的键/默认值 → 去掉注解后最后两个词是「类型 变量名」。
pub fn parse(params: &str) -> Vec<Param> {
    let inner = params.trim().trim_start_matches('(').trim_end_matches(')');
    split_top_level(inner).into_iter().filter_map(|piece| parse_one(&piece)).collect()
}

/// 参数：`params` 为解析结果，`class_text` 按类名取类文本（对象参数展开用）。
/// 核心流程：跳过 `@RequestBody` 参数 → 键取 `@RequestParam` 名 → 值依次取注解默认值、
/// 基础类型占位值（US-E3-02：数值 `0`、布尔 `false`、字符串空串）、对象字段占位值（至多 3 层）
/// → 用 `&` 拼接；解析不出的对象按 U6 给 `null` 占位，没有参数返回空串。
pub fn query_string(params: &[Param], class_text: impl Fn(&str) -> Option<String>) -> String {
    let mut pairs: Vec<String> = Vec::new();
    for param in params.iter().filter(|p| !p.body) {
        if let Some(default) = &param.default {
            pairs.push(format!("{}={}", param.key, encode(default)));
            continue;
        }
        if let Some(default) = query_default(&param.ty) {
            pairs.push(format!("{}={}", param.key, encode(default)));
            continue;
        }
        if !expand_object(&param.ty, &class_text, 0, &mut pairs) {
            pairs.push(format!("{}=null", param.key));
        }
    }
    pairs.dedup();
    pairs.join("&")
}

/// 参数：`piece` 为单个参数片段。
/// 核心流程：标记 `@RequestBody` → 取 `@RequestParam` 的首个引号串与 `defaultValue` →
/// 去掉全部注解后按最后一个空白切成（类型，变量名）。
fn parse_one(piece: &str) -> Option<Param> {
    let body = piece.contains("@RequestBody");
    let key = request_param_name(piece);
    let default = default_value(piece);
    let plain = strip_annotations(piece);
    let (ty, name) = plain.rsplit_once(char::is_whitespace)?;
    let name = name.trim();
    let ty = ty.trim();
    if name.is_empty() || ty.is_empty() {
        return None;
    }
    Some(Param {
        name: name.to_string(),
        ty: ty.to_string(),
        body,
        key: key.unwrap_or_else(|| name.to_string()),
        default,
    })
}

/// 参数：`piece` 为单个参数片段。
/// 核心流程：找 `@RequestParam`，只在 `defaultValue` 之前找第一个引号串作键名，
/// 这样 `@RequestParam(defaultValue = "7") int page` 的键仍是变量名。
fn request_param_name(piece: &str) -> Option<String> {
    let at = piece.find("@RequestParam")?;
    let args = &piece[at..];
    let end = args.find(')').unwrap_or(args.len());
    let before_default = args[..end].split("defaultValue").next().unwrap_or(&args[..end]);
    quoted(before_default)
}

/// 参数：`piece` 为单个参数片段。
/// 核心流程：找 `defaultValue` 之后的第一个引号串作为参数默认值。
fn default_value(piece: &str) -> Option<String> {
    let at = piece.find("defaultValue")?;
    quoted(&piece[at..])
}

/// 参数：`text` 为一段含引号的文本。
/// 核心流程：取第一个 `"` 与随后的 `"` 之间的内容；缺任一端返回 None。
fn quoted(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_string())
}

/// 参数：`piece` 为单个参数片段。
/// 核心流程：反复删掉 `@Name(...)` 或 `@Name`，剩下的就是「类型 变量名」。
fn strip_annotations(piece: &str) -> String {
    let mut rest = piece.to_string();
    while let Some(at) = rest.find('@') {
        let tail = &rest[at..];
        let end = match tail.find('(') {
            Some(open) => {
                let mut depth = 0usize;
                let mut end = tail.len();
                for (i, ch) in tail.char_indices().skip(open) {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                at + end
            }
            None => at + tail.find(char::is_whitespace).unwrap_or(tail.len()),
        };
        rest.replace_range(at..end.min(rest.len()), " ");
    }
    rest.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 参数：`text` 为顶层参数列表。
/// 核心流程：按 `,` 切分，但跳过 `<...>`、`(...)`、`[...]` 内部的分隔符。
fn split_top_level(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out.into_iter().filter(|p| !p.trim().is_empty()).collect()
}

/// 参数：`ty` 为类型原文。
/// 核心流程：US-E3-02 的口径——数值给 `0`、布尔给 `false`、字符串/字符/日期给空串
/// （输出 `k=` 形式）；其它类型返回 None，表示要按对象展开。
fn query_default(ty: &str) -> Option<&'static str> {
    Some(match ty.trim().to_lowercase().as_str() {
        "byte" | "short" | "int" | "integer" | "long" | "double" | "float" | "bigdecimal" | "biginteger" => "0",
        "boolean" | "bool" => "false",
        "string" | "char" | "character" | "date" | "localdate" | "localdatetime" | "datetime" | "timestamp" => "",
        _ => return None,
    })
}

/// 参数：`ty` 为类型原文，`class_text` 按类名取文本，`depth` 为已展开层数，`pairs` 为输出。
/// 核心流程：查类文本 → 逐字段：基础类型记 `字段=占位值`，对象字段递归（至多 3 层）；
/// 返回是否展开了任何字段，供调用方决定要不要用 `null` 占位。
fn expand_object(ty: &str, class_text: &dyn Fn(&str) -> Option<String>, depth: usize, pairs: &mut Vec<String>) -> bool {
    if depth >= 3 {
        return false;
    }
    let Some(name) = element_type(ty) else { return false };
    let Some(text) = class_text(&name) else { return false };
    let mut expanded = false;
    for field in fields_of(&text) {
        if let Some(default) = query_default(&field.ty) {
            pairs.push(format!("{}={}", field.name, encode(default)));
            expanded = true;
        } else if expand_object(&field.ty, class_text, depth + 1, pairs) {
            expanded = true;
        }
    }
    expanded
}

/// 参数：`ty` 为类型原文（可能是 `List<PageQuery>`、`PageQuery[]`）。
/// 核心流程：剥掉数组与泛型外壳，取元素类型的类名；基础类型或空壳返回 None。
fn element_type(ty: &str) -> Option<String> {
    let ty = ty.trim().trim_end_matches("[]").trim();
    let inner = match (ty.find('<'), ty.rfind('>')) {
        (Some(open), Some(close)) if close > open => ty[open + 1..close].trim(),
        _ => ty,
    };
    (!inner.is_empty() && query_default(inner).is_none()).then(|| inner.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(name: &str) -> Option<String> {
        match name {
            "PageQuery" => Some("public class PageQuery {\n    static final long serialVersionUID = 1L;\n    int pageNo;\n    int pageSize;\n}\n".into()),
            "Address" => Some("public class Address {\n    String city;\n}\n".into()),
            "Profile" => Some("public class Profile {\n    int level;\n    Address address;\n}\n".into()),
            _ => None,
        }
    }

    #[test]
    fn no_params_gives_an_empty_string() {
        assert_eq!(query_string(&parse("()"), classes), "");
        assert_eq!(query_string(&parse(""), classes), "");
    }

    #[test]
    fn base_types_get_sample_values() {
        let params = parse("(int age, long id, String name, boolean vip, Date day)");
        assert_eq!(
            query_string(&params, classes),
            "age=0&id=0&name=&vip=false&day="
        );
    }

    #[test]
    fn request_body_params_are_skipped() {
        let params = parse("(String name, int age, @RequestBody User body)");
        assert_eq!(params.len(), 3);
        assert!(params[2].body);
        assert_eq!(query_string(&params, classes), "name=&age=0");
    }

    #[test]
    fn object_params_expand_to_their_base_fields() {
        let params = parse("(PageQuery query)");
        assert_eq!(params[0].ty, "PageQuery");
        assert_eq!(query_string(&params, classes), "pageNo=0&pageSize=0");
    }

    #[test]
    fn nested_objects_expand_two_levels() {
        let params = parse("(Profile profile)");
        assert_eq!(query_string(&params, classes), "level=0&city=");
    }

    #[test]
    fn request_param_renames_the_key_and_takes_defaults() {
        let params = parse(r#"(@RequestParam("q") String keyword, @RequestParam(defaultValue = "7") int page)"#);
        assert_eq!(params[0].key, "q");
        assert_eq!(query_string(&params, classes), "q=&page=7");
    }

    #[test]
    fn values_with_spaces_and_symbols_are_percent_encoded() {
        let params = parse(r#"(@RequestParam(defaultValue = "a b&c") String q)"#);
        assert_eq!(query_string(&params, classes), "q=a%20b%26c");
    }

    #[test]
    fn generic_commas_do_not_split_parameters() {
        let params = parse("(Map<String, Object> body, int page)");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].ty, "Map<String, Object>");
        assert_eq!(query_string(&params, classes), "body=null&page=0", "解析不出的对象按 U6 给 null 占位");
    }
}
