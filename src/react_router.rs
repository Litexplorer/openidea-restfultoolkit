//! Vite + React 路由扫描（react-router）：JSX `<Route>` 声明。
//!
//! 纯文本扫描，不依赖 TS/JSX 解析器：逐字符跳过注释与字符串，只在路由声明处取值。

use crate::scan::{Endpoint, HttpMethod};
use crate::url;

/// 路由声明标记：文本里出现任一才值得扫描（廉价预筛）。
const MARKERS: [&str; 5] =
    ["<Route", "createBrowserRouter(", "createRoutesFromElements(", "useRoutes(", "@react-router/dev/routes"];

/// 参数：`text` 为前端源码全文。
/// 核心流程：出现任一 JSX/对象/v7 配置 DSL 标记才返回 true。
pub fn looks_like_route_file(text: &str) -> bool {
    MARKERS.iter().any(|marker| text.contains(marker))
}

/// 参数：`text` 为 .tsx/.jsx/.ts/.js 源码全文。
/// 核心流程：JSX `<Route>`、data router 对象数组、v7 `route()/index()` 配置各扫一遍。
pub fn scan(text: &str) -> Vec<Endpoint> {
    if !looks_like_route_file(text) {
        return Vec::new();
    }
    let lines = LineIndex::new(text);
    let mut out = Vec::new();
    scan_jsx_routes(text, &lines, &mut out);
    scan_route_objects(text, &lines, &mut out);
    scan_route_dsl(text, &lines, &mut out);
    out
}

/// 行号索引：把字节偏移换算成 1 基行号。
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    /// 参数：`text` 为源码全文。
    /// 核心流程：记录每行起点（首行 0 与每个换行后的位置）。
    fn new(text: &str) -> Self {
        let mut starts = vec![0usize];
        starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self { starts }
    }

    /// 参数：`pos` 为字节偏移。
    /// 核心流程：二分最后一个不大于 `pos` 的行起点，返回 1 基行号。
    fn line(&self, pos: usize) -> u32 {
        match self.starts.binary_search(&pos) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }
}

/// 参数：`text` 与引号起始位置 `open`。
/// 核心流程：按引号类型扫到配对的收尾引号，返回收尾之后的位置；未闭合返回末尾。
fn skip_string(text: &str, open: usize) -> usize {
    let quote = text.as_bytes()[open];
    let mut i = open + 1;
    while i < text.len() {
        let b = text.as_bytes()[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote {
            return i + 1;
        }
        i += 1;
    }
    text.len()
}

/// 参数：`text` 与当前位置 `i`。
/// 核心流程：命中 `//` 或 `/*` 时返回注释之后的位置，否则返回 None。
fn skip_comment(text: &str, i: usize) -> Option<usize> {
    if text[i..].starts_with("//") {
        return Some(text[i..].find('\n').map_or(text.len(), |n| i + n));
    }
    if text[i..].starts_with("/*") {
        return Some(text[i + 2..].find("*/").map_or(text.len(), |n| i + 2 + n + 2));
    }
    None
}

/// 参数：`text` 与 `<` 的字节位置 `start`。
/// 核心流程：跳过字符串、注释与成对括号，返回标签收尾 `>` 之后的位置。
fn find_tag_end(text: &str, start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut depth = 0usize;
    while i < text.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let b = text.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(text, i);
            continue;
        }
        match b {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            b'>' if depth == 0 => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// 参数：`text` 与左括号位置 `open`。
/// 核心流程：按同类括号配平，返回右括号之后的位置。
fn balanced_end(text: &str, open: usize) -> Option<usize> {
    let open_b = *text.as_bytes().get(open)?;
    let close_b = match open_b {
        b'{' => b'}',
        b'(' => b')',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0usize;
    let mut i = open;
    while i < text.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let b = text.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(text, i);
            continue;
        }
        if b == open_b {
            depth += 1;
        } else if b == close_b {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// 一个 JSX 属性（值保留原文，可能带引号或花括号）。
struct Attr {
    name: String,
    value: String,
}

/// 参数：标签起始位置与字节。
/// 核心流程：属性名只由字母、数字、`_`、`$`、`-` 组成。
fn is_attr_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

/// 参数：同上。
fn is_attr_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'-'
}

/// 参数：`text` 与位置 `i`。
/// 核心流程：跳过空白，返回第一个非空白字节位置。
fn skip_ws(text: &str, mut i: usize) -> usize {
    while i < text.len() {
        let ch = text[i..].chars().next().unwrap_or(' ');
        if !ch.is_whitespace() {
            break;
        }
        i += ch.len_utf8();
    }
    i
}

/// 参数：`text` 与值起点 `start`。
/// 核心流程：引号值读到收尾引号，花括号值读到配平，裸值读到空白或标签结尾。
fn attr_value(text: &str, start: usize) -> (String, usize) {
    let Some(&first) = text.as_bytes().get(start) else { return (String::new(), start) };
    if first == b'"' || first == b'\'' {
        let end = skip_string(text, start);
        let inner_end = end.saturating_sub(1).max(start + 1);
        return (text[start + 1..inner_end].to_string(), end);
    }
    if first == b'{' {
        let end = balanced_end(text, start).unwrap_or(text.len());
        return (text[start..end].to_string(), end);
    }
    let mut i = start;
    while i < text.len() {
        let b = text.as_bytes()[i];
        if b.is_ascii_whitespace() || b == b'>' || (b == b'/' && text[i..].starts_with("/>")) {
            break;
        }
        i += 1;
    }
    (text[start..i].to_string(), i)
}

/// 参数：`tag` 为完整标签文本（`<Route ...>`）。
/// 核心流程：跳过标签名后逐字符读取顶层属性，括号/引号内的内容不算属性边界。
fn tag_attrs(tag: &str) -> Vec<Attr> {
    let mut out = Vec::new();
    let mut i = "<Route".len();
    let mut depth = 0usize;
    while i < tag.len() {
        if let Some(next) = skip_comment(tag, i) {
            i = next;
            continue;
        }
        let b = tag.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(tag, i);
            continue;
        }
        match b {
            b'{' | b'(' | b'[' => {
                depth += 1;
                i += 1;
                continue;
            }
            b'}' | b')' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth > 0 || !is_attr_start(b) {
            i += 1;
            continue;
        }
        let name_start = i;
        while i < tag.len() && is_attr_char(tag.as_bytes()[i]) {
            i += 1;
        }
        let name = tag[name_start..i].to_string();
        let after_name = skip_ws(tag, i);
        if tag.as_bytes().get(after_name) == Some(&b'=') {
            let (value, next) = attr_value(tag, skip_ws(tag, after_name + 1));
            out.push(Attr { name, value });
            i = next;
        } else {
            out.push(Attr { name, value: String::new() });
            i = after_name;
        }
    }
    out
}

/// 参数：`value` 为属性值原文。
/// 核心流程：剥掉一层引号；花括号表达式里再剥引号或反引号；模板串含 `${` 视为未解析。
fn path_literal(value: &str) -> Option<(String, bool)> {
    let raw = value.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(inner) = quoted(raw) {
        return Some((inner, true));
    }
    let expr = raw.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')).map(str::trim).unwrap_or(raw);
    if let Some(inner) = quoted(expr) {
        return Some((inner, true));
    }
    if let Some(inner) = backtick(expr) {
        let resolved = !inner.contains("${");
        return Some((inner, resolved));
    }
    Some((expr.to_string(), false))
}

/// 参数：`text` 为一段文本。
/// 核心流程：首尾是同一对单双引号时返回中间内容。
fn quoted(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && (bytes[0] == b'"' || bytes[0] == b'\'') && bytes[bytes.len() - 1] == bytes[0] {
        return Some(text[1..text.len() - 1].to_string());
    }
    None
}

/// 参数：`text` 为一段文本。
/// 核心流程：首尾是反引号时返回中间内容。
fn backtick(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    (bytes.len() >= 2 && bytes[0] == b'`' && bytes[bytes.len() - 1] == b'`').then(|| text[1..text.len() - 1].to_string())
}

/// 参数：`value` 为布尔属性原文。
/// 核心流程：无值或 `{true}` 算 true，其余算 false。
fn attr_true(value: &str) -> bool {
    let raw = value.trim();
    if raw.is_empty() {
        return true;
    }
    let inner = raw.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')).map(str::trim).unwrap_or(raw);
    inner == "true"
}

/// 参数：`parent` 为父级前缀，`raw` 为子路由路径原文。
/// 核心流程：绝对路径重置前缀，相对路径接在父级后面，最后统一归一化。
fn resolve_route_path(parent: &str, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return url::normalize_path(parent);
    }
    if raw.starts_with('/') {
        return url::normalize_path(raw);
    }
    url::normalize_path(&format!("{parent}/{raw}"))
}

/// 参数：属性表。
/// 核心流程：优先取 `element`/`Component` 里的组件名，其次取 `lazy` 的 import 模块名。
fn component_of(attrs: &[Attr]) -> Option<String> {
    for name in ["element", "Component"] {
        if let Some(attr) = attrs.iter().find(|attr| attr.name == name) {
            if let Some(found) = component_name(&attr.value) {
                return Some(found);
            }
        }
    }
    attrs.iter().find(|attr| attr.name == "lazy").and_then(|attr| import_stem(&attr.value))
}

/// 参数：`value` 为 JSX 表达式原文。
/// 核心流程：取 `<` 之后的首个大写标识符（组件名）。
fn component_name(value: &str) -> Option<String> {
    let at = value.find('<')? + 1;
    let name: String = value[at..]
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '$' || *ch == '.')
        .collect();
    name.chars().next().filter(|ch| ch.is_uppercase()).map(|_| name)
}

/// 参数：`value` 为 `lazy` 属性原文。
/// 核心流程：取 `import("...")` 里的模块文件名（去扩展名）作展示名。
fn import_stem(value: &str) -> Option<String> {
    let start = value.find("import(")? + "import(".len();
    let rest = &value[start..];
    let quote_at = rest.find(['"', '\'', '`'])?;
    let end = skip_string(rest, quote_at);
    let literal = &rest[quote_at + 1..end.saturating_sub(1)];
    let file = literal.rsplit('/').next().unwrap_or(literal);
    let stem = file.split('.').next().unwrap_or(file);
    (!stem.is_empty()).then(|| stem.to_string())
}

/// 参数：`text` 为源码全文，`lines` 为行号索引，`out` 为结果。
/// 核心流程：顺序扫描标签；`<Route>` 入栈、`</Route>` 出栈；自闭合标签不入栈。
fn scan_jsx_routes(text: &str, lines: &LineIndex, out: &mut Vec<Endpoint>) {
    let mut stack: Vec<String> = vec![String::new()];
    let mut i = 0usize;
    while i < text.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let b = text.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(text, i);
            continue;
        }
        if text[i..].starts_with("</Route") {
            if stack.len() > 1 {
                stack.pop();
            }
            i += "</Route".len();
            continue;
        }
        if text[i..].starts_with("<Route") {
            let after = i + "<Route".len();
            let boundary = text[after..].chars().next().is_none_or(|ch| ch.is_whitespace() || ch == '>' || ch == '/');
            if boundary {
                let Some(end) = find_tag_end(text, i) else { break };
                let tag = &text[i..end];
                let attrs = tag_attrs(tag);
                let parent = stack.last().cloned().unwrap_or_default();
                let indexed = attrs.iter().find(|attr| attr.name == "index").is_some_and(|attr| attr_true(&attr.value));
                let literal = attrs.iter().find(|attr| attr.name == "path").and_then(|attr| path_literal(&attr.value));
                let (path, unresolved, emit) = if indexed {
                    (url::normalize_path(&parent), false, true)
                } else if let Some((raw, resolved)) = literal {
                    (resolve_route_path(&parent, &raw), !resolved, true)
                } else {
                    (url::normalize_path(&parent), false, false)
                };
                if emit {
                    push_react(out, path.clone(), component_of(&attrs).unwrap_or_default(), None, lines.line(i), unresolved);
                }
                if !tag.trim_end().ends_with("/>") {
                    stack.push(path);
                }
                i = end;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// 对象式路由：`createBrowserRouter([...])` 里的一个对象字面量。
struct RouterObject {
    start: usize,
    end: usize,
    /// 外层对象在 `objects` 里的下标；顶层数组元素为 None。
    parent: Option<usize>,
}

/// 参数：`text` 为源码全文，`lines` 为行号索引，`out` 为结果。
/// 核心流程：只在 `createBrowserRouter(`/`useRoutes(` 的参数里找对象数组，避免误收 API 配置。
fn scan_route_objects(text: &str, lines: &LineIndex, out: &mut Vec<Endpoint>) {
    let mut i = 0usize;
    while i < text.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let b = text.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(text, i);
            continue;
        }
        let marker = ["createBrowserRouter(", "useRoutes("].into_iter().find(|marker| text[i..].starts_with(marker));
        if let Some(marker) = marker {
            let open = i + marker.len() - 1;
            if let Some(end) = balanced_end(text, open) {
                parse_object_routes(&text[open + 1..end - 1], open + 1, lines, out);
                i = end;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// 参数：`slice` 为路由数组的参数文本，`offset` 为该文本在文件中的字节偏移。
/// 核心流程：先收集对象树（父对象先生成）→ 逐对象按父级前缀解析路径 → 有 path/index
/// 且带渲染字段（或位于顶层）才产出端点。
fn parse_object_routes(slice: &str, offset: usize, lines: &LineIndex, out: &mut Vec<Endpoint>) {
    let objects = object_spans(slice);
    let mut prefixes: Vec<String> = vec![String::new(); objects.len()];
    for (idx, object) in objects.iter().enumerate() {
        let inner = &slice[object.start + 1..object.end.saturating_sub(1)];
        let parent_prefix = object.parent.map_or_else(String::new, |parent| prefixes[parent].clone());
        let literal = object_field(inner, "path").and_then(|value| path_literal(&value));
        let indexed = object_field(inner, "index").is_some_and(|value| attr_true(&value));
        let has_render = ["element", "Component", "lazy", "children"]
            .iter()
            .any(|key| object_field(inner, key).is_some());
        let (path, unresolved, emit) = if indexed {
            (url::normalize_path(&parent_prefix), false, true)
        } else if let Some((raw, resolved)) = literal {
            (resolve_route_path(&parent_prefix, &raw), !resolved, true)
        } else {
            (parent_prefix.clone(), false, false)
        };
        if emit && (has_render || object.parent.is_none()) {
            push_react(out, path.clone(), object_component(inner).unwrap_or_default(), None, lines.line(offset + object.start), unresolved);
        }
        prefixes[idx] = path;
    }
}

/// 参数：`slice` 为路由数组的参数文本。
/// 核心流程：扫一遍花括号，开括号时登记对象（父节点取当前栈顶），闭括号时补结束位置。
fn object_spans(slice: &str) -> Vec<RouterObject> {
    let mut objects: Vec<RouterObject> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < slice.len() {
        if let Some(next) = skip_comment(slice, i) {
            i = next;
            continue;
        }
        let b = slice.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(slice, i);
            continue;
        }
        if b == b'{' {
            objects.push(RouterObject { start: i, end: 0, parent: stack.last().copied() });
            stack.push(objects.len() - 1);
        } else if b == b'}' {
            if let Some(idx) = stack.pop() {
                objects[idx].end = i + 1;
            }
        }
        i += 1;
    }
    for object in &mut objects {
        if object.end == 0 {
            object.end = slice.len();
        }
    }
    objects
}

/// 参数：`inner` 为一个对象字面量内部的文本，`key` 为字段名。
/// 核心流程：只认对象顶层的 `key:`；键可带单双引号；返回值原文（可能含引号或花括号）。
fn object_field(inner: &str, key: &str) -> Option<String> {
    let mut i = 0usize;
    let mut depth = 0usize;
    while i < inner.len() {
        if let Some(next) = skip_comment(inner, i) {
            i = next;
            continue;
        }
        let b = inner.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            let end = skip_string(inner, i);
            if depth == 0 && b != b'`' && &inner[i + 1..end.saturating_sub(1)] == key {
                let after = skip_ws(inner, end);
                if inner.as_bytes().get(after) == Some(&b':') {
                    return field_value(inner, skip_ws(inner, after + 1));
                }
            }
            i = end;
            continue;
        }
        match b {
            b'{' | b'(' | b'[' => {
                depth += 1;
                i += 1;
                continue;
            }
            b'}' | b')' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth == 0 && is_attr_start(b) {
            let name_start = i;
            while i < inner.len() && is_attr_char(inner.as_bytes()[i]) {
                i += 1;
            }
            if &inner[name_start..i] == key {
                let after = skip_ws(inner, i);
                if inner.as_bytes().get(after) == Some(&b':') {
                    return field_value(inner, skip_ws(inner, after + 1));
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

/// 参数：`text` 与字段值起点。
/// 核心流程：引号/括号值按配对收尾，裸值读到顶层逗号或换行，返回去空白后的原文。
fn field_value(text: &str, start: usize) -> Option<String> {
    let first = *text.as_bytes().get(start)?;
    if first == b'"' || first == b'\'' {
        let end = skip_string(text, start);
        return Some(text[start..end].to_string());
    }
    if first == b'{' || first == b'[' || first == b'(' {
        let end = balanced_end(text, start).unwrap_or(text.len());
        return Some(text[start..end].to_string());
    }
    let mut i = start;
    let mut depth = 0usize;
    while i < text.len() {
        let b = text.as_bytes()[i];
        match b {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' | b'\n' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    Some(text[start..i].trim().to_string())
}

/// 参数：`inner` 为路由对象内部文本。
/// 核心流程：`element`/`Component` 先取组件名，其次 `lazy` 的 import 模块名。
fn object_component(inner: &str) -> Option<String> {
    for key in ["element", "Component"] {
        let Some(value) = object_field(inner, key) else { continue };
        if let Some(found) = component_name(&value) {
            return Some(found);
        }
        let bare = value.trim().trim_matches(|ch| ch == '{' || ch == '}').trim();
        if bare.chars().next().is_some_and(|ch| ch.is_uppercase()) {
            return Some(bare.to_string());
        }
    }
    object_field(inner, "lazy").and_then(|value| import_stem(&value))
}

/// 参数：`out` 为结果，`path`/`handler`/`module`/`line`/`unresolved` 为一条 React 路由。
/// 核心流程：统一填成无 HTTP 动词、`react=true` 的端点；`module` 仅 v7 配置有值。
fn push_react(
    out: &mut Vec<Endpoint>,
    path: String,
    handler: String,
    module: Option<String>,
    line: u32,
    unresolved: bool,
) {
    out.push(Endpoint {
        http_method: HttpMethod::RequestMapping,
        path,
        handler,
        line,
        unresolved,
        jaxrs: false,
        fastapi: false,
        react: true,
        module,
    });
}

/// `@react-router/dev/routes` 的配置 DSL 函数名。
const ROUTE_DSL: [&str; 4] = ["route", "index", "prefix", "layout"];

/// 配置 DSL 的嵌套上限，防病态输入把调用栈打穿。
const DSL_MAX_DEPTH: u8 = 16;

/// 参数：`text` 为源码全文，`lines` 为行号索引，`out` 为结果。
/// 核心流程：只处理引了 `@react-router/dev/routes` 的文件，从文件顶层开始递归解析。
fn scan_route_dsl(text: &str, lines: &LineIndex, out: &mut Vec<Endpoint>) {
    if !text.contains("@react-router/dev/routes") {
        return;
    }
    parse_dsl_range(text, 0, text.len(), "", false, 0, lines, out);
}

/// 参数：`[start, end)` 为待扫描区间，`parent` 为父级前缀，`unresolved_parent` 表示前缀没解析出来。
/// 核心流程：逐字符跳过注释与字符串 → 命中 `route/index/prefix/layout` 调用就处理并跳过整个调用。
fn parse_dsl_range(
    text: &str,
    start: usize,
    end: usize,
    parent: &str,
    unresolved_parent: bool,
    depth: u8,
    lines: &LineIndex,
    out: &mut Vec<Endpoint>,
) {
    if depth > DSL_MAX_DEPTH {
        return;
    }
    let mut i = start;
    while i < end {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let b = text.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(text, i);
            continue;
        }
        if is_attr_start(b) {
            let name_start = i;
            while i < end && is_attr_char(text.as_bytes()[i]) {
                i += 1;
            }
            let name = &text[name_start..i];
            let open = skip_ws(text, i);
            if ROUTE_DSL.contains(&name) && text.as_bytes().get(open) == Some(&b'(') {
                if let Some(call_end) = balanced_end(text, open) {
                    emit_dsl_call(text, name, open, call_end, parent, unresolved_parent, depth, lines, out);
                    i = call_end;
                    continue;
                }
            }
            continue;
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// 参数：`name` 为 DSL 函数名，`open`/`call_end` 为括号区间（`call_end` 在右括号之后）。
/// 核心流程：按函数语义取路径/模块/子数组；`route` 产出自己再下钻 children，
/// `index` 产出父路径，`prefix` 只换前缀，`layout` 只透传前缀。
#[allow(clippy::too_many_arguments)]
fn emit_dsl_call(
    text: &str,
    name: &str,
    open: usize,
    call_end: usize,
    parent: &str,
    unresolved_parent: bool,
    depth: u8,
    lines: &LineIndex,
    out: &mut Vec<Endpoint>,
) {
    let args = call_args(text, open, call_end.saturating_sub(1));
    let line = lines.line(open);
    match name {
        "index" => {
            let module = module_arg(text, args.first().copied());
            let handler = module.as_deref().map(module_label).unwrap_or_default();
            push_react(out, url::normalize_path(parent), handler, module, line, unresolved_parent);
        }
        "route" => {
            let Some(path_span) = args.first().copied() else { return };
            let (raw, resolved) = literal_at(text, path_span)
                .unwrap_or_else(|| (text[path_span.0..path_span.1].trim().to_string(), false));
            let path = resolve_route_path(parent, &raw);
            let unresolved = unresolved_parent || !resolved;
            let module = module_arg(text, args.get(1).copied());
            let handler = module.as_deref().map(module_label).unwrap_or_default();
            push_react(out, path.clone(), handler, module, line, unresolved);
            if let Some(range) = args.get(2).and_then(|span| array_range(text, *span)) {
                parse_dsl_range(text, range.0, range.1, &path, unresolved, depth + 1, lines, out);
            }
        }
        "prefix" => {
            let Some(span) = args.first().copied() else { return };
            let (child_parent, unresolved) = match literal_at(text, span) {
                Some((raw, resolved)) => (resolve_route_path(parent, &raw), unresolved_parent || !resolved),
                None => (url::normalize_path(parent), true),
            };
            if let Some(range) = args.get(1).and_then(|span| array_range(text, *span)) {
                parse_dsl_range(text, range.0, range.1, &child_parent, unresolved, depth + 1, lines, out);
            }
        }
        "layout" => {
            if let Some(range) = args.get(1).and_then(|span| array_range(text, *span)) {
                parse_dsl_range(text, range.0, range.1, parent, unresolved_parent, depth + 1, lines, out);
            }
        }
        _ => {}
    }
}

/// 参数：`text` 与括号区间（`close` 为右括号下标）。
/// 核心流程：按顶层逗号切参数，跳过字符串与括号，返回去空白的字节区间。
fn call_args(text: &str, open: usize, close: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = open + 1;
    let mut i = start;
    let mut depth = 0usize;
    while i < close {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let b = text.as_bytes()[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_string(text, i);
            continue;
        }
        match b {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                out.push(trim_span(text, (start, i)));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = trim_span(text, (start, close));
    if last.0 < last.1 || out.is_empty() {
        out.push(last);
    }
    out
}

/// 参数：`text` 与字节区间。
/// 核心流程：去掉区间两端的 ASCII 空白。
fn trim_span(text: &str, (mut start, mut end): (usize, usize)) -> (usize, usize) {
    while start < end && text.as_bytes()[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && text.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start, end)
}

/// 参数：`text` 与一个参数区间。
/// 核心流程：字面量参数返回（内容，是否可静态解析）；数组与动态表达式返回 None。
fn literal_at(text: &str, span: (usize, usize)) -> Option<(String, bool)> {
    let (start, end) = trim_span(text, span);
    if start >= end {
        return None;
    }
    let raw = &text[start..end];
    if raw.starts_with('[') {
        return None;
    }
    path_literal(raw)
}

/// 参数：`text` 与一个参数区间。
/// 核心流程：区间是数组字面量时返回其内部 `[start, end)`，否则 None。
fn array_range(text: &str, span: (usize, usize)) -> Option<(usize, usize)> {
    let (start, end) = trim_span(text, span);
    if text.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let close = balanced_end(text, start)?.min(end);
    Some((start + 1, close.saturating_sub(1)))
}

/// 参数：`text` 与模块参数区间（第二个字符串参数）。
/// 核心流程：取引号/反引号内容；非字面量返回 None。
fn module_arg(text: &str, span: Option<(usize, usize)>) -> Option<String> {
    let (start, end) = span.map(|span| trim_span(text, span))?;
    if start >= end {
        return None;
    }
    let raw = text[start..end].trim();
    quoted(raw).or_else(|| backtick(raw))
}

/// 参数：`module` 为 v7 模块参数原文。
/// 核心流程：去 `./` 与扩展名，作为树的展示名。
fn module_label(module: &str) -> String {
    let path = module.trim_start_matches("./");
    path.rsplit_once('.').map_or(path, |(base, _)| base).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(text: &str) -> Vec<String> {
        scan(text).into_iter().map(|endpoint| endpoint.path).collect()
    }

    #[test]
    fn a_single_route_is_found() {
        let text = "export function App() {\n  return (\n    <Routes>\n      <Route path=\"/users\" element={<Users />} />\n    </Routes>\n  );\n}\n";
        let found = scan(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/users");
        assert_eq!(found[0].handler, "Users");
        assert_eq!(found[0].line, 4);
        assert!(found[0].react && !found[0].jaxrs && !found[0].fastapi);
        assert_eq!(found[0].http_method, HttpMethod::RequestMapping);
    }

    #[test]
    fn a_file_without_route_markers_is_skipped() {
        assert!(scan("export const x = 1;\n").is_empty());
        assert!(!looks_like_route_file("export const x = 1;\n"));
    }

    #[test]
    fn routes_inside_comments_and_strings_are_ignored() {
        let text = "// <Route path=\"/gone\" />\nconst s = \"<Route path='/also-gone' />\";\n<Route path=\"/kept\" />\n";
        assert_eq!(paths(text), ["/kept"]);
    }

    #[test]
    fn nested_routes_join_the_parent_path() {
        let text = "\
<Route path=\"/admin\" element={<Layout />}>
  <Route path=\"users\" element={<Users />} />
  <Route path=\"roles\" element={<Roles />} />
</Route>
";
        assert_eq!(paths(text), ["/admin", "/admin/users", "/admin/roles"]);
    }

    #[test]
    fn a_child_with_a_leading_slash_is_absolute() {
        let text = "<Route path=\"/admin\"><Route path=\"/login\" element={<Login />} /></Route>\n";
        assert_eq!(paths(text), ["/admin", "/login"]);
    }

    #[test]
    fn index_routes_take_the_parent_path() {
        let text = "\
<Route path=\"/admin\">
  <Route index element={<Home />} />
  <Route path=\"users\" element={<Users />} />
</Route>
<Route index element={<Root />} />
";
        assert_eq!(paths(text), ["/admin", "/admin", "/admin/users", "/"]);
    }

    #[test]
    fn index_false_is_not_a_route() {
        let text = "<Route path=\"/admin\"><Route index={false} element={<Home />} /></Route>\n";
        assert_eq!(paths(text), ["/admin"]);
    }

    #[test]
    fn multi_line_tags_and_single_quotes_are_supported() {
        let text = "\
<Route
  path='/reports'
  element={<Reports />}
>
  <Route
    path={'daily'}
    element={<Daily />}
  />
</Route>
";
        assert_eq!(paths(text), ["/reports", "/reports/daily"]);
    }

    #[test]
    fn dynamic_paths_are_marked_unresolved() {
        let text = "<Route path={ROUTES.user} element={<User />} />\n<Route path={`/orgs/${org}/x`} element={<X />} />\n<Route path={\"/static\"} element={<S />} />\n";
        let found = scan(text);
        assert_eq!(found.len(), 3);
        assert!(found[0].unresolved, "常量表达式未解析");
        assert!(found[1].unresolved, "模板串占位符未解析");
        assert!(!found[2].unresolved, "花括号里的字面量可解析");
        assert_eq!(found[2].path, "/static");
    }

    #[test]
    fn lazy_imports_supply_the_display_name() {
        let text = "<Route path=\"/users\" lazy={() => import('./pages/Users')} />\n";
        assert_eq!(scan(text)[0].handler, "Users");
    }

    #[test]
    fn the_routes_element_is_not_a_route() {
        let text = "<Routes><Route path=\"/x\" element={<X />} /></Routes>\n";
        assert_eq!(paths(text), ["/x"]);
    }

    #[test]
    fn data_router_objects_are_scanned() {
        let text = "\
const router = createBrowserRouter([
  {
    path: '/',
    element: <Layout />,
    children: [
      { index: true, element: <Home /> },
      { path: 'users', element: <Users /> },
      { path: 'users/:id', element: <UserDetail /> },
    ],
  },
  { path: '/login', element: <Login /> },
]);
";
        let found = scan(text);
        let found_paths: Vec<&str> = found.iter().map(|endpoint| endpoint.path.as_str()).collect();
        assert_eq!(found_paths, ["/", "/", "/users", "/users/:id", "/login"]);
        assert_eq!(found[1].handler, "Home");
        assert_eq!(found[2].handler, "Users");
        assert_eq!(found[0].line, 2, "行号指向对象起始行");
        assert!(found.iter().all(|endpoint| endpoint.react));
    }

    #[test]
    fn use_routes_arrays_are_scanned() {
        let text = "const routes = useRoutes([{ path: 'reports', element: <Reports /> }]);\n";
        assert_eq!(paths(text), ["/reports"]);
        assert_eq!(scan(text)[0].handler, "Reports");
    }

    #[test]
    fn handle_objects_do_not_become_routes() {
        let text = "createBrowserRouter([{ path: '/a', element: <A />, handle: { path: '/nope' } }]);\n";
        assert_eq!(paths(text), ["/a"]);
    }

    #[test]
    fn index_false_objects_are_not_routes() {
        let text = "createBrowserRouter([{ path: '/a', element: <A />, children: [{ index: false, element: <B /> }] }]);\n";
        assert_eq!(paths(text), ["/a"]);
    }

    #[test]
    fn dynamic_object_paths_are_marked_unresolved() {
        let found = scan("createBrowserRouter([{ path: ROUTES.user, element: <User /> }]);\n");
        assert_eq!(found.len(), 1);
        assert!(found[0].unresolved);
    }

    #[test]
    fn objects_outside_a_router_call_are_ignored() {
        let text = "const api = { path: '/not-a-route', element: null };\n";
        assert!(paths(text).is_empty());
    }

    #[test]
    fn commented_out_data_routers_are_ignored() {
        let text = "\
// createBrowserRouter([{ path: '/gone', element: <Gone /> }]);
/* const r = useRoutes([{ path: '/also-gone', element: <X /> }]); */
createBrowserRouter([{ path: '/kept', element: <Kept /> }]);
";
        assert_eq!(paths(text), ["/kept"]);
    }

    const V7_ROUTES: &str = r#"import { type RouteConfig, index, layout, prefix, route } from "@react-router/dev/routes";
export default [
  route("/", "routes/_layout.tsx", [
    index("routes/index.tsx"),
    route("clipped", "routes/clipped/index.tsx"),
    route("playback/:liveId/:displayOrder", "routes/playback/$liveId._layout.tsx", [
      index("routes/playback/$liveId.tsx"),
      route("clipped", "routes/playback/$liveId.clipped.tsx"),
    ]),
  ]),
  prefix("api", [
    route("health", "routes/health.tsx"),
  ]),
  layout("routes/layout.tsx", [
    route("settings", "routes/settings.tsx"),
  ]),
] satisfies RouteConfig;
"#;

    #[test]
    fn the_v7_route_config_dsl_is_scanned() {
        let found = scan(V7_ROUTES);
        let found_paths: Vec<&str> = found.iter().map(|endpoint| endpoint.path.as_str()).collect();
        assert_eq!(
            found_paths,
            [
                "/",
                "/",
                "/clipped",
                "/playback/:liveId/:displayOrder",
                "/playback/:liveId/:displayOrder",
                "/playback/:liveId/:displayOrder/clipped",
                "/api/health",
                "/settings",
            ]
        );
        let handlers: Vec<&str> = found.iter().map(|endpoint| endpoint.handler.as_str()).collect();
        assert_eq!(
            handlers,
            [
                "routes/_layout",
                "routes/index",
                "routes/clipped/index",
                "routes/playback/$liveId._layout",
                "routes/playback/$liveId",
                "routes/playback/$liveId.clipped",
                "routes/health",
                "routes/settings",
            ]
        );
        assert_eq!(found[0].line, 3, "行号指向 route() 所在行");
        assert!(found.iter().all(|endpoint| endpoint.react && !endpoint.unresolved));
    }

    #[test]
    fn v7_dynamic_paths_are_marked_unresolved() {
        let text = "import { route } from \"@react-router/dev/routes\";\nexport default [route(PATHS.users, \"routes/users.tsx\")];\n";
        let found = scan(text);
        assert_eq!(found.len(), 1);
        assert!(found[0].unresolved);
    }

    #[test]
    fn a_route_call_without_the_v7_import_is_ignored() {
        let text = "export default [route(\"users\", \"routes/users.tsx\")];\n";
        assert!(paths(text).is_empty(), "没有 @react-router/dev/routes 标记不解析");
    }

    #[test]
    fn a_dynamic_prefix_marks_its_children_unresolved() {
        let text = "import { prefix, route } from \"@react-router/dev/routes\";\nexport default [prefix(API_BASE, [route(\"health\", \"routes/health.tsx\")])];\n";
        let found = scan(text);
        assert_eq!(found.len(), 1);
        assert!(found[0].unresolved);
    }
}
