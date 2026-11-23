//! 类文本 → 示例 JSON（US-E3-03）：纯文本字段扫描，不依赖 PSI。
//!
//! 基础类型给示例值，集合给一个元素，嵌套对象递归展开，自关联与超过
//! `MAX_DEPTH` 的递归按 `{}` 截断（visited 集合随递归路径传递，不用任何全局状态）。

use std::collections::{BTreeMap, BTreeSet};

/// 递归深度上限（设置 `json.maxDepth` 没给时的缺省值）。
pub const MAX_DEPTH: usize = 4;

/// 生成选项：`maxDepth` 与 `skipSerialVersionUid` 来自设置，每次调用现传，不缓存。
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub max_depth: usize,
    pub skip_serial: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { max_depth: MAX_DEPTH, skip_serial: true }
    }
}

/// 字段的修饰符，出现在类型前时要丢掉。
const MODIFIERS: &[&str] = &[
    "private", "public", "protected", "final", "transient", "volatile", "static", "readonly",
];

/// 类的一个字段：名字与类型原文（泛型与数组原样保留）。
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: String,
}

/// 参数：`ty` 为类型原文。
/// 核心流程：小写比对基础类型表，命中给示例值（查询串里不加引号），否则 None。
pub fn base_default(ty: &str) -> Option<&'static str> {
    Some(match ty.trim().to_lowercase().as_str() {
        "byte" | "short" | "int" | "integer" | "long" | "double" | "float" | "bigdecimal" | "biginteger" => "1",
        "boolean" | "bool" => "true",
        "char" | "character" => "Z",
        "string" => "demoData",
        "date" | "localdate" | "localdatetime" | "datetime" | "timestamp" => "2024-01-01 00:00:00",
        _ => return None,
    })
}

/// 参数：`ty` 为类型原文。
/// 核心流程：判断示例值写进 JSON 时要不要加引号（字符串、字符与日期要）。
fn is_text(ty: &str) -> bool {
    matches!(
        ty.trim().to_lowercase().as_str(),
        "string" | "char" | "character" | "date" | "localdate" | "localdatetime" | "datetime" | "timestamp"
    )
}

/// 参数：`class_text` 为类声明文本（Java）。
/// 核心流程：逐行跳过注释、注解、方法与类声明行 → 去掉修饰符与初始值 →
/// 取末尾标识符为字段名、前一个词为类型；`static` 常量与 `serialVersionUID` 不入表。
pub fn fields_of(class_text: &str) -> Vec<Field> {
    fields_of_with(class_text, true)
}

/// 参数：`class_text` 为类声明文本，`skip_serial` 为是否跳过 `serialVersionUID`（设置 `skipSerialVersionUid`）。
/// 核心流程：与 `fields_of` 相同，只是跳过项由调用方决定。
pub fn fields_of_with(class_text: &str, skip_serial: bool) -> Vec<Field> {
    let mut out = Vec::new();
    for raw in class_text.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with('@')
            || line.contains('(')
            || line.contains(')')
            || line.contains(" class ")
            || line.contains(" interface ")
            || line.contains(" enum ")
            || line.starts_with("class ")
        {
            continue;
        }
        let decl = line.split('=').next().unwrap_or(line).trim_end_matches(';').trim();
        let tokens: Vec<&str> = decl.split_whitespace().collect();
        if tokens.len() < 2 {
            continue;
        }
        let name = tokens[tokens.len() - 1];
        let ty = tokens[tokens.len() - 2];
        let is_static = tokens[..tokens.len() - 2].contains(&"static");
        if MODIFIERS.contains(&ty) {
            continue;
        }
        // serialVersionUID 默认跳过；关掉跳过项时按普通字段保留（即使它是 static final）。
        if name == "serialVersionUID" {
            if skip_serial {
                continue;
            }
        } else if is_static {
            continue;
        }
        out.push(Field { name: name.to_string(), ty: ty.to_string() });
    }
    out
}

/// 参数：`ty` 为类型原文，`lookup` 按类名取类文本，`visited` 为当前递归路径上的类名，
/// `depth` 为当前缩进层级（同时充当递归深度）。
/// 核心流程：数组/List/Set 给 `[]`、Map 给 `{}`（US-E5-01 占位值表）→ 基础类型给字面量 →
/// 其余按类名查文本递归缩进；查不到类给 `null`，重名（自关联）或超 `MAX_DEPTH` 给 `{}`。
fn value(
    ty: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    visited: &mut BTreeSet<String>,
    depth: usize,
    options: Options,
) -> String {
    let ty = ty.trim();
    let lower = ty.to_lowercase();
    if lower.starts_with("map<") || lower == "map" {
        return "{}".to_string();
    }
    if ty.ends_with("[]") || ["list<", "set<", "collection<", "iterable<"].iter().any(|head| lower.starts_with(head)) {
        return "[]".to_string();
    }
    if let Some(default) = base_default(ty) {
        return if is_text(ty) { format!("\"{default}\"") } else { default.to_string() };
    }
    let Some(text) = lookup(ty) else { return "null".to_string() };
    if depth >= options.max_depth || !visited.insert(ty.to_string()) {
        return "{}".to_string();
    }
    let json = object_json(&text, lookup, visited, depth, options);
    visited.remove(ty);
    json
}

/// 参数：`class_text` 为类文本，`lookup` 同 `value`，`visited`/`depth` 为递归状态。
/// 核心流程：解析字段 → 逐字段生成 JSON 值并用 2 空格缩进 → 字段顺序与源码声明一致；空类给 `{}`。
fn object_json(
    class_text: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    visited: &mut BTreeSet<String>,
    depth: usize,
    options: Options,
) -> String {
    let fields = fields_of_with(class_text, options.skip_serial);
    if fields.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::from("{\n");
    for (index, field) in fields.iter().enumerate() {
        let value = value(&field.ty, lookup, visited, depth + 1, options);
        out.push_str(&format!("{}\"{}\": {}", "  ".repeat(depth + 1), field.name, value));
        if index + 1 < fields.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(&"  ".repeat(depth));
    out.push('}');
    out
}

/// 参数：`ty` 为类型名（`UserCreateReq`、`String`、`List<User>`、`Map<String,Object>` 等），
/// `lookup` 按类名取类文本。
/// 核心流程：按对象/集合/基础类型分派生成一个缩进 JSON 值，作为 `@RequestBody` 的骨架。
pub fn json_for(ty: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    json_for_with(ty, lookup, Options::default())
}

/// 参数：`ty` 为类型名，`lookup` 按类名取类文本，`options` 为 `maxDepth` / `skipSerialVersionUid`。
/// 核心流程：与 `json_for` 相同，深度与跳过项改由设置决定。
pub fn json_for_with(ty: &str, lookup: impl Fn(&str) -> Option<String>, options: Options) -> String {
    value(ty, &lookup, &mut BTreeSet::new(), 0, options)
}

/// 参数：`class_text` 为类声明文本，`lookup` 按类名取类文本（嵌套字段用）。
/// 核心流程：先把类自己的名字放进 visited（自关联第一层就截断）→ 生成 JSON 对象；空类给 `{}`。
pub fn class_json(class_text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    class_json_with(class_text, lookup, Options::default())
}

/// 参数：`class_text` 为类声明文本，`lookup` 按类名取类文本，`options` 为 `maxDepth` / `skipSerialVersionUid`。
/// 核心流程：与 `class_json` 相同，深度与跳过项改由设置决定。
pub fn class_json_with(class_text: &str, lookup: impl Fn(&str) -> Option<String>, options: Options) -> String {
    let mut visited = BTreeSet::new();
    if let Some(name) = class_name_of(class_text) {
        visited.insert(name);
    }
    object_json(class_text, &lookup, &mut visited, 0, options)
}

/// 参数：`class_text` 为类声明文本。
/// 核心流程：取 `class ` 之后的第一个标识符作为类名；没有声明返回 None。
fn class_name_of(class_text: &str) -> Option<String> {
    let rest = &class_text[class_text.find("class ")? + "class ".len()..];
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

/// 参数：`json` 为（可能已缩进的）JSON 文本。
/// 核心流程：`serde_json` 解析后用 `to_string` 压成单行；不是合法 JSON 时原样返回，不 panic。
pub fn to_compressed(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_else(|| json.to_string())
}

/// 参数：`class_text` 为类声明文本，`parent` 为直接父类文本（可缺省），`line_ending` 为行尾。
/// 核心流程：转成一条链交给 `to_bulk_value_full`；缺省 CRLF、`Options::default()`。
pub fn to_bulk_value_with(class_text: &str, parent: Option<&str>, line_ending: &str) -> String {
    let parents: Vec<String> = parent.map(str::to_string).into_iter().collect();
    to_bulk_value_full(class_text, &parents, line_ending, Options::default())
}

/// 参数：`class_text` 为类声明文本，`parents` 为祖先文本（第 0 个是直接父类，往后逐层向上），
/// `line_ending` 为行尾，`options` 为生成选项。
/// 核心流程：最顶层祖先的字段排在最前、逐层向下，子类同名字段沿用父类位置但取子类声明 →
/// 每行 `字段名:占位值`（冒号后不留空格）→ 用给定行尾连接，结尾不留空行。
pub fn to_bulk_value_full(
    class_text: &str,
    parents: &[String],
    line_ending: &str,
    options: Options,
) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    for parent in parents.iter().rev() {
        for field in fields_of_with(parent, options.skip_serial) {
            if !order.contains(&field.name) {
                order.push(field.name.clone());
            }
            types.entry(field.name).or_insert(field.ty);
        }
    }
    for field in fields_of_with(class_text, options.skip_serial) {
        if !order.contains(&field.name) {
            order.push(field.name.clone());
        }
        types.insert(field.name, field.ty);
    }
    order
        .iter()
        .map(|name| format!("{}:{}", name, base_default(types.get(name).map_or("", String::as_str)).unwrap_or("demoData")))
        .collect::<Vec<_>>()
        .join(line_ending)
}

/// 参数：`class_text` 为类声明文本。
/// 核心流程：无父类、CRLF 行尾的 `to_bulk_value_with`（US-E5-03 的内置缺省）。
pub fn to_bulk_value(class_text: &str) -> String {
    to_bulk_value_with(class_text, None, "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty(_: &str) -> Option<String> {
        None
    }

    fn cls(body: &str) -> String {
        format!("public class X {{\n    {body}\n}}\n")
    }

    fn named(name: &str, body: &str) -> String {
        format!("public class {name} {{\n    {body}\n}}\n")
    }

    #[test]
    fn empty_class_is_an_empty_object() {
        assert_eq!(class_json("public class Empty {\n}\n", empty), "{}");
        assert_eq!(class_json("", empty), "{}");
    }

    #[test]
    fn base_fields_are_indented_with_two_spaces_in_declaration_order() {
        let text = "public class User {\n    private int age;\n    private String name;\n    private boolean vip;\n}\n";
        assert_eq!(
            class_json(text, empty),
            "{\n  \"age\": 1,\n  \"name\": \"demoData\",\n  \"vip\": true\n}"
        );
    }

    #[test]
    fn collections_and_maps_follow_the_placeholder_table() {
        let text = cls("List<Tag> tags;\n    String[] codes;\n    Map<String,Object> extra;");
        assert_eq!(
            class_json(&text, empty),
            "{\n  \"tags\": [],\n  \"codes\": [],\n  \"extra\": {}\n}",
            "US-E5-01：List/Set/数组给 []，Map 给 {{}}"
        );
        assert_eq!(json_for("List<Tag>", empty), "[]");
    }

    #[test]
    fn nested_objects_are_expanded_with_indentation() {
        let lookup = |name: &str| (name == "Address").then(|| cls("String city;"));
        let text = cls("String name;\n    Address address;");
        assert_eq!(
            class_json(&text, lookup),
            "{\n  \"name\": \"demoData\",\n  \"address\": {\n    \"city\": \"demoData\"\n  }\n}"
        );
    }

    #[test]
    fn self_reference_stops_at_the_visited_set() {
        let text = named("Node", "Node child;\n    int value;");
        let lookup = |name: &str| (name == "Node").then(|| text.clone());
        assert_eq!(class_json(&text, lookup), "{\n  \"child\": {},\n  \"value\": 1\n}");
    }

    #[test]
    fn recursion_stops_at_the_documented_depth_of_four() {
        assert_eq!(MAX_DEPTH, 4, "US-E5-01：maxDepth 默认 4");
        let lookup = |name: &str| match name {
            "A" => Some(cls("B b;")),
            "B" => Some(cls("C c;")),
            "C" => Some(cls("D d;")),
            "D" => Some(cls("E e;")),
            "E" => Some(cls("int v;")),
            _ => None,
        };
        assert_eq!(
            json_for("A", lookup),
            "{\n  \"b\": {\n    \"c\": {\n      \"d\": {\n        \"e\": {}\n      }\n    }\n  }\n}"
        );
    }

    #[test]
    fn unknown_types_give_null_and_strings_quote_their_placeholder() {
        assert_eq!(json_for("Map<String,Object>", empty), "{}");
        assert_eq!(json_for("String", empty), r#""demoData""#);
        assert_eq!(json_for("UserCreateReq", empty), "null", "查不到类给 null（US-E5-01）");
    }

    #[test]
    fn static_and_constant_fields_are_skipped() {
        let text = cls("static final long serialVersionUID = 1L;\n    private static int COUNT = 0;\n    int pageNo;");
        assert_eq!(class_json(&text, empty), "{\n  \"pageNo\": 1\n}");
    }

    #[test]
    fn compressed_json_matches_the_pretty_one_semantically() {
        let text = cls("String name;\n    int age;");
        let pretty = class_json(&text, empty);
        let compressed = to_compressed(&pretty);
        assert!(!compressed.contains('\n'), "{compressed}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&compressed).unwrap(),
            serde_json::from_str::<serde_json::Value>(&pretty).unwrap()
        );
        assert_eq!(to_compressed("not json"), "not json");
    }

    #[test]
    fn bulk_value_uses_crlf_parent_fields_first_and_demo_data() {
        let parent = cls("int id;");
        let child = cls("String name;\n    List<String> tags;");
        assert_eq!(to_bulk_value_with(&child, Some(&parent), "\r\n"), "id:1\r\nname:demoData\r\ntags:demoData");
        assert_eq!(to_bulk_value(&child), "name:demoData\r\ntags:demoData", "缺省行尾是 CRLF");
        assert_eq!(to_bulk_value_with(&child, Some(&parent), "\n"), "id:1\nname:demoData\ntags:demoData");
    }

    #[test]
    fn bulk_value_keeps_the_child_declaration_of_a_shared_field() {
        let parent = cls("int id;\n    String name;");
        let child = cls("long id;");
        assert_eq!(to_bulk_value_with(&child, Some(&parent), "\n"), "id:1\nname:demoData");
    }

    #[test]
    fn bulk_value_walks_every_parent_level() {
        let grandparent = cls("long tenantId;");
        let parent = cls("int id;");
        let child = cls("String name;");
        let chain = vec![parent, grandparent];
        assert_eq!(
            to_bulk_value_full(&child, &chain, "\n", Options::default()),
            "tenantId:1\nid:1\nname:demoData",
            "最顶层祖先的字段最先出现"
        );
    }
}
