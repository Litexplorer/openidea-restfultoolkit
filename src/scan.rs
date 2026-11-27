//! Spring / JAX-RS / Kotlin 源码扫描（US-E1-01 / US-E1-04）。纯文本扫描，注释里的伪注解跳过。
//!
//! 数组路径与多动词取笛卡尔积；无法静态解析的路径（常量拼接、`${...}` 占位符、裸变量）
//! 照样产出端点，但把 `unresolved` 置为 true，由上层给出提示。

#[derive(Clone, Debug, PartialEq)]
pub enum HttpMethod { Get, Post, Put, Delete, Patch, Head, Options, RequestMapping }

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub http_method: HttpMethod,
    pub path: String,
    pub handler: String,
    pub line: u32,
    /// 路径没能静态解析（常量、拼接、`${...}` 占位符），`path` 里保留原文。
    pub unresolved: bool,
    /// 来自 JAX-RS 注解（`@Path` + `@GET` 等），索引里 framework 写 `jaxrs`。
    pub jaxrs: bool,
    /// 来自 FastAPI 装饰器（`@router.get` 等），索引里 framework 写 `fastapi`。
    pub fastapi: bool,
    /// 来自 React 路由声明（`<Route>`、`createBrowserRouter` 等），索引里 framework 写 `react-router`。
    pub react: bool,
    /// v7 `route(path, module, ...)` 的模块参数（相对 app 目录），索引据此生成第二跳位置。
    pub module: Option<String>,
}

/// 参数：`text` 为 Java 源码全文。
/// 核心流程：取首个含 `RequestMapping` 的行 → 解析括号内第一个双引号串；没有则空串。
pub fn class_base_path(text: &str) -> String {
    text.lines().find(|l| l.contains("RequestMapping")).and_then(first_quoted).unwrap_or_default()
}

/// 参数：`text` 为 Java 源码全文，行的编号按 1 基计。
/// 核心流程：跳注释与类型声明 → 行首 Spring 映射注解累积成待定端点（路径与动词都可多个）→
/// 随后的方法声明行按「路径 × 动词」笛卡尔积产出端点。
pub fn scan_file(text: &str) -> Vec<Endpoint> {
    if feign_only(text) {
        return Vec::new();
    }
    scan_annotated(text, false)
}

/// 参数：`text` 为源码全文。
/// 核心流程：有 `@FeignClient` 又没有 `@RestController`/`@Controller`/`@Path` 时，
/// 说明这是调用别人的客户端接口，不算本项目的端点。
fn feign_only(text: &str) -> bool {
    text.contains("@FeignClient")
        && !["@RestController", "@Controller", "@Path"].iter().any(|marker| text.contains(marker))
}

/// 参数：`text` 为 Kotlin 源码全文。
/// 核心流程：与 `scan_file` 同一套注解逻辑，方法名取 `fun` 之后的第一个标识符。
pub fn scan_kotlin(text: &str) -> Vec<Endpoint> {
    if feign_only(text) {
        return Vec::new();
    }
    scan_annotated(text, true)
}

/// 参数：`path` 为文件路径，`text` 为全文。
/// 核心流程：`.py` 走 FastAPI、前端扩展名走 React，其余按 `.kt`/Java 走 Spring 并上 JAX-RS；
/// 同一文件含两套后端框架时各出一组，互不混合。
pub fn scan_any(path: &str, text: &str) -> Vec<Endpoint> {
    if path.ends_with(".py") {
        return scan_fastapi(text);
    }
    if is_frontend_path(path) {
        return crate::react_router::scan(text);
    }
    let spring = if path.ends_with(".kt") { scan_kotlin(text) } else { scan_file(text) };
    spring.into_iter().chain(scan_jaxrs(text)).collect()
}

/// 参数：`path` 为文件路径。
/// 核心流程：前端扩展名（ts/tsx/js/jsx）走 React 路由扫描。
pub fn is_frontend_path(path: &str) -> bool {
    [".tsx", ".jsx", ".ts", ".js"].iter().any(|ext| path.ends_with(ext))
}

/// FastAPI 支持的 HTTP 动词（装饰器方法名）。
const FASTAPI_VERBS: [&str; 7] = ["get", "post", "put", "delete", "patch", "head", "options"];

/// 参数：`text` 为 Python 源码全文。
/// 核心流程：先收同文件的 `var = APIRouter(prefix=...)` → 逐行找 `@<receiver>.<verb>(...)`
/// （支持跨行、`path=` 关键字、无参数）→ 路径 = prefix + 装饰器路径 → 取其后最近的
/// `def`/`async def` 作方法名，行号为装饰器行（1 基）；receiver 不在 prefix 表里标 unresolved。
pub fn scan_fastapi(text: &str) -> Vec<Endpoint> {
    scan_fastapi_with(text, None)
}

/// 前缀图查询：`receiver` → 完整前缀。
pub type PrefixResolver<'a> = Option<&'a dyn Fn(&str) -> Option<String>>;

/// 参数：`text` 为 Python 源码全文，`resolver` 为前缀图查询（`None` 表示只按本文件解析）。
/// 核心流程：门禁与列 0 规则同 `scan_fastapi`；前缀优先取 `resolver(file, receiver)`，
/// 查不到时退回本文件 `APIRouter(prefix=)` 并标 unresolved（保持相对路径）。
pub fn scan_fastapi_with(text: &str, resolver: PrefixResolver<'_>) -> Vec<Endpoint> {
    // FastAPI 门禁：没有 `from fastapi` / `import fastapi` 的文件一律不算（aiohttp 的 @routes.get 会误判）。
    if !is_fastapi_file(text) {
        return Vec::new();
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let prefixes = router_prefixes(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut used = 0usize;
    while used < lines.len() {
        let raw = lines[used];
        used += 1;
        let decorator_line = used as u32;
        // 列 0 门禁：生产装饰器都在行首；tests 里缩进的 fixture 不算路由。
        let Some(rest) = raw.strip_prefix('@') else { continue };
        let Some(mut call) = decorator_call(rest, &lines[used..]) else { continue };
        used += call.consumed;
        let (prefix, unresolved) = match resolver {
            Some(resolve) => match resolve(&call.receiver) {
                Some(full) => (full, false),
                None => (prefixes.get(&call.receiver).cloned().unwrap_or_default(), true),
            },
            None => match prefixes.get(&call.receiver) {
                Some(prefix) => (prefix.clone(), false),
                None => (String::new(), true),
            },
        };
        let path = crate::url::join_url("", &prefix, &call.path);
        let (handler, def_offset) = match find_def(&lines[used..]) {
            Some(found) => found,
            None => continue,
        };
        // 行号：装饰器所在行（1 基）；拿不到时退回 def 行。
        call.line = if decorator_line > 0 { decorator_line } else { used as u32 + def_offset as u32 + 1 };
        out.push(Endpoint {
            http_method: call.method,
            path,
            handler,
            line: call.line,
            unresolved,
            jaxrs: false,
            fastapi: true,
            react: false,
            module: None,
        })
    }
    out
}

/// 参数：`text` 为文件全文。
/// 核心流程：只有出现 `from fastapi` 或 `import fastapi` 才算 FastAPI 文件。
pub fn is_fastapi_file(text: &str) -> bool {
    text.contains("from fastapi") || text.contains("import fastapi")
}

/// 参数：`text` 为文件全文。
/// 核心流程：数行首（列 0）的 `@<ident>.websocket(` 个数——本期跳过但要能在 diagnostics 里看到。
pub fn websocket_decorators(text: &str) -> usize {
    text.lines().filter(|line| line.starts_with('@') && line.contains(".websocket(")).count()
}

/// 参数：`text` 为文件全文。
/// 核心流程：FastAPI 文件且存在行首 `@<ident>.<verb>(`（verb 含 websocket）才值得解析。
pub fn looks_like_fastapi(text: &str) -> bool {
    const VERBS: [&str; 8] = ["get", "post", "put", "delete", "patch", "head", "options", "websocket"];
    is_fastapi_file(text)
        && text.lines().any(|line| {
            line.starts_with('@') && VERBS.iter().any(|verb| line.contains(&format!(".{verb}(")))
        })
}

/// 一个 FastAPI 装饰器调用拆出来的信息。
struct DecoratorCall {
    receiver: String,
    method: HttpMethod,
    path: String,
    line: u32,
    /// 装饰器跨了几行（后续扫描从这之后开始）。
    consumed: usize,
}

/// 参数：`text` 为 Python 源码全文。
/// 核心流程：逐行找 `var = APIRouter(`，取等号左边变量名与括号里 `prefix=` 的第一个字符串字面量。
fn router_prefixes(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some(at) = line.find("APIRouter(") else { continue };
        let Some((left, _)) = line[..at].split_once('=') else { continue };
        let name = left.trim().to_string();
        let args = &line[at + "APIRouter(".len()..];
        if let Some(prefix) = first_string(args) {
            map.insert(name, prefix);
        }
    }
    map
}

/// 参数：`rest` 为 `@` 之后的文本，`following` 为后续行。
/// 核心流程：拆出 receiver 与动词；括号内容可跨行累积到配平为止；路径取第一个字符串字面量。
fn decorator_call(rest: &str, following: &[&str]) -> Option<DecoratorCall> {
    let (head, tail, consumed) = match rest.find('(') {
        Some(open) => {
            let head = &rest[..open];
            let (args, consumed) = collect_args(&rest[open + 1..], following)?;
            (head, args, consumed)
        }
        None => (rest.split_whitespace().next().unwrap_or(rest), String::new(), 0),
    };
    let (receiver, verb) = head.rsplit_once('.')?;
    let verb = verb.trim().to_lowercase();
    if !FASTAPI_VERBS.contains(&verb.as_str()) {
        return None;
    }
    let method = match verb.as_str() {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "patch" => HttpMethod::Patch,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        _ => return None,
    };
    Some(DecoratorCall {
        receiver: receiver.trim().to_string(),
        method,
        path: first_string(&tail).unwrap_or_default(),
        line: 0,
        consumed,
    })
}

/// 参数：`first` 为 `(` 之后的第一段，`following` 为后续行。
/// 核心流程：累加直到括号配平，返回（括号内文本，额外吃掉的行数）。
fn collect_args(first: &str, following: &[&str]) -> Option<(String, usize)> {
    let mut round = 1i32;
    let mut square = 0i32;
    let mut curly = 0i32;
    let mut args = String::new();
    let mut consumed = 0usize;
    let mut pending = first;
    loop {
        let mut in_string: Option<char> = None;
        let mut chars = pending.chars().peekable();
        while let Some(ch) = chars.next() {
            if let Some(quote) = in_string {
                args.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        args.push(next);
                    }
                } else if ch == quote {
                    in_string = None;
                }
                continue;
            }
            match ch {
                '"' | '\'' => {
                    in_string = Some(ch);
                    args.push(ch);
                }
                '(' => {
                    round += 1;
                    args.push(ch);
                }
                ')' => {
                    round -= 1;
                    if round == 0 {
                        return Some((args, consumed));
                    }
                    args.push(ch);
                }
                '[' => {
                    square += 1;
                    args.push(ch);
                }
                ']' => {
                    square -= 1;
                    args.push(ch);
                }
                '{' => {
                    curly += 1;
                    args.push(ch);
                }
                '}' => {
                    curly -= 1;
                    args.push(ch);
                }
                _ => args.push(ch),
            }
        }
        let _ = (square, curly);
        if consumed >= following.len() {
            return Some((args, consumed));
        }
        args.push(' ');
        pending = following[consumed];
        consumed += 1;
    }
}

/// 参数：`text` 为一段 Python 参数文本。
/// 核心流程：取第一个字符串字面量（单双引号都支持），`path="..."` 与位置参数都走这条。
fn first_string(text: &str) -> Option<String> {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '"' && ch != '\'' {
            continue;
        }
        let quote = ch;
        let mut out = String::new();
        let mut escaped = false;
        for next in chars.by_ref() {
            if escaped {
                out.push(next);
                escaped = false;
                continue;
            }
            if next == '\\' {
                escaped = true;
                continue;
            }
            if next == quote {
                return Some(out);
            }
            out.push(next);
        }
        return Some(out);
    }
    None
}

/// 参数：`lines` 为装饰器之后的行。
/// 核心流程：找最近的 `def name(` 或 `async def name(`，返回（名字，相对行偏移）；
/// 越过下一个装饰器或 class 就停，越界返回 None。
fn find_def(lines: &[&str]) -> Option<(String, usize)> {
    for (offset, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("async ").unwrap_or(trimmed);
        if let Some(after) = rest.strip_prefix("def ") {
            let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() {
                return Some((name, offset));
            }
        }
        if trimmed.starts_with("class ") || trimmed.starts_with('@') {
            break;
        }
    }
    None
}

/// Spring 映射注解的待定端点：同一方法上可能先后来多条注解。
#[derive(Default)]
struct Pending {
    methods: Vec<HttpMethod>,
    paths: Vec<String>,
    line: u32,
    unresolved: bool,
}

impl Pending {
    /// 参数：`other` 为后来的注解。
    /// 核心流程：动词取并集、路径后写覆盖、unresolved 取或、行号保留最早的注解行。
    fn merge(&mut self, other: Pending) {
        for method in other.methods {
            if !self.methods.contains(&method) {
                self.methods.push(method);
            }
        }
        if !other.paths.is_empty() {
            self.paths = other.paths;
        }
        self.unresolved |= other.unresolved;
        if self.line == 0 {
            self.line = other.line;
        }
    }

    /// 参数：`handler` 为方法名。
    /// 核心流程：路径（缺省空串）× 动词各产出一条端点，顺序与注解里一致。
    fn emit(self, handler: &str) -> Vec<Endpoint> {
        let paths = if self.paths.is_empty() { vec![String::new()] } else { self.paths };
        let methods = if self.methods.is_empty() { vec![HttpMethod::RequestMapping] } else { self.methods };
        let mut out = Vec::new();
        for method in &methods {
            for path in &paths {
                out.push(Endpoint {
                    http_method: method.clone(),
                    path: path.clone(),
                    handler: handler.to_string(),
                    line: self.line,
                    unresolved: self.unresolved,
                    jaxrs: false,
                    fastapi: false,
                    react: false,
                    module: None,
                });
            }
        }
        out
    }
}

/// 参数：`text` 为源码全文，`kotlin` 为是否按 Kotlin 的方法声明取值。
/// 核心流程：逐行跳 `//` 与 `/* */` 注释、类型声明（清空待定）→ 映射注解累积进 pending →
/// 方法声明行产出端点并清空 pending。
fn scan_annotated(text: &str, kotlin: bool) -> Vec<Endpoint> {
    let mut out = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut in_block = false;
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if in_block || line.starts_with("/*") {
            in_block = !line.contains("*/");
            continue;
        }
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if is_type_line(line) {
            pending = None;
            continue;
        }
        if line.starts_with('@') {
            if let Some((name, args)) = parse_annotation(line) {
                if let Some(found) = spring_pending(name, args, idx as u32 + 1) {
                    let mut next = found;
                    if let Some(mut current) = pending.take() {
                        current.merge(next);
                        next = current;
                    }
                    pending = Some(next);
                }
            }
            continue;
        }
        let handler = if kotlin { kotlin_handler(line) } else { handler_name(line) };
        if let Some(handler) = handler {
            if let Some(pending) = pending.take() {
                out.extend(pending.emit(&handler));
            }
        }
    }
    out
}

/// 参数：`text` 为 JAX-RS 源码全文（Java 或 Kotlin）。
/// 核心流程：逐行跳注释与类型声明 → 收集方法级 `@Path` 与 `@GET` 等动词注解 →
/// 遇到首处 `标识符(` 的声明行即按「路径 × 动词」产出 `Endpoint`（line 为注解行，1 基）。
pub fn scan_jaxrs(text: &str) -> Vec<Endpoint> {
    if feign_only(text) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let (mut methods, mut paths, mut line_no) = (Vec::new(), Vec::new(), 0u32);
    let mut unresolved = false;
    let mut in_block = false;
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if in_block || line.starts_with("/*") {
            in_block = !line.contains("*/");
            continue;
        }
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if is_type_line(line) {
            methods.clear();
            paths.clear();
            line_no = 0;
            unresolved = false;
            continue;
        }
        if let Some(at) = line.find('@') {
            let name = annotation_short_name(&line[at + 1..]);
            if name == "Path" {
                let (found, unsure) = value_paths(args_of(&line[at..]));
                paths = found;
                unresolved |= unsure;
            } else if let Some(method) = jaxrs_method(name) {
                if !methods.contains(&method) {
                    methods.push(method);
                }
            } else {
                continue;
            }
            if line_no == 0 {
                line_no = idx as u32 + 1;
            }
            continue;
        }
        if methods.is_empty() {
            continue;
        }
        if let Some(handler) = declared_name(line) {
            let annotation_line = if line_no == 0 { idx as u32 + 1 } else { line_no };
            let found = if paths.is_empty() { vec![String::new()] } else { std::mem::take(&mut paths) };
            for method in &methods {
                for path in &found {
                    out.push(Endpoint {
                        http_method: method.clone(),
                        path: path.clone(),
                        handler: handler.clone(),
                        line: annotation_line,
                        unresolved,
                        jaxrs: true,
                        fastapi: false,
                        react: false,
                        module: None,
                    });
                }
            }
            methods.clear();
            line_no = 0;
            unresolved = false;
        }
    }
    out
}

/// 参数：`text` 为源码全文。
/// 核心流程：取第一个类型声明行之前、含 `@Path` 的首行，取其第一个引号串作 JAX-RS 类级路径。
pub fn jaxrs_class_path(text: &str) -> String {
    for line in text.lines() {
        if is_type_line(line) {
            break;
        }
        if line.contains("@Path") {
            return first_quoted(line).unwrap_or_default();
        }
    }
    String::new()
}

/// 参数：`name` 为注解名，`args` 为括号内文本，`line` 为注解所在行（1 基）。
/// 核心流程：只认六个 Spring 映射注解；路径与动词都可能是多个，`unresolved` 随路径一起返回。
fn spring_pending(name: &str, args: &str, line: u32) -> Option<Pending> {
    let methods = spring_methods(annotation_short_name(name), args)?;
    let (paths, unresolved) = value_paths(args);
    Some(Pending { methods, paths, line, unresolved })
}

/// 参数：`name` 为注解简名，`args` 为括号内文本。
/// 核心流程：五个动词注解直接命中；`@RequestMapping` 按 GET/POST/PUT/DELETE/PATCH 顺序
/// 收集 `method` 属性里出现的全部动词，未写明则保留 `RequestMapping`。
fn spring_methods(name: &str, args: &str) -> Option<Vec<HttpMethod>> {
    let direct = [("GetMapping", HttpMethod::Get), ("PostMapping", HttpMethod::Post),
        ("PutMapping", HttpMethod::Put), ("DeleteMapping", HttpMethod::Delete),
        ("PatchMapping", HttpMethod::Patch)];
    if let Some((_, method)) = direct.into_iter().find(|(key, _)| *key == name) {
        return Some(vec![method]);
    }
    if name != "RequestMapping" {
        return None;
    }
    let verbs = [("RequestMethod.GET", HttpMethod::Get), ("RequestMethod.POST", HttpMethod::Post),
        ("RequestMethod.PUT", HttpMethod::Put), ("RequestMethod.DELETE", HttpMethod::Delete),
        ("RequestMethod.PATCH", HttpMethod::Patch)];
    let found: Vec<HttpMethod> = verbs.into_iter().filter(|(key, _)| args.contains(key)).map(|(_, m)| m).collect();
    Some(if found.is_empty() { vec![HttpMethod::RequestMapping] } else { found })
}

/// 参数：`args` 为映射注解括号内文本。
/// 核心流程：只认 `value =`/`path =` 属性或首个位置参数——其余属性（`headers`、`produces`、
/// `consumes`、`name`…）里的引号串一律不当路径；取到原文却不是字面量时标记未解析。
fn value_paths(args: &str) -> (Vec<String>, bool) {
    let Some(slot) = path_slot(args) else { return (vec![String::new()], false) };
    let quoted = quoted_values(&slot.text);
    if quoted.is_empty() {
        let raw = slot
            .text
            .trim_matches(|c: char| c.is_whitespace() || c == '{' || c == '}' || c == ')')
            .trim()
            .to_string();
        return if raw.is_empty() { (vec![String::new()], false) } else { (vec![raw], true) };
    }
    let unresolved = has_concat(&slot.text) || quoted.iter().any(|path| path.contains("${"));
    (quoted, unresolved)
}

/// 路径所在的那一段文本。
struct Slot {
    text: String,
}

/// 参数：`args` 为注解括号内文本。
/// 核心流程：按顶层逗号切成若干「属性段」→ 优先 `value`/`path` 属性；没有这两个属性时，
/// 只有第一段是位置参数（以引号或 `{` 开头，或只有一个词）才当作路径，否则返回 None。
fn path_slot(args: &str) -> Option<Slot> {
    let segments = split_args(args);
    for segment in &segments {
        let trimmed = segment.trim();
        for key in ["value", "path"] {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim_start();
                if let Some(value) = rest.strip_prefix('=') {
                    return Some(Slot { text: value.trim().to_string() });
                }
            }
        }
    }
    let first = segments.first()?.trim().to_string();
    // 位置参数：以引号/花括号开头，或者是没有顶层 `=` 的裸表达式（常量、拼接、变量）。
    let positional = first.starts_with('"') || first.starts_with('{') || !has_top_level_equals(&first);
    (positional && !first.is_empty()).then_some(Slot { text: first })
}

/// 参数：`text` 为一段注解参数。
/// 核心流程：判断有没有顶层的 `=`（不在字符串与括号里），用来区分「位置参数」与「命名属性」。
fn has_top_level_equals(text: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if in_string {
            if ch == '\\' {
                chars.next();
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            '=' if depth <= 0 => return true,
            _ => {}
        }
    }
    false
}

/// 参数：`args` 为注解括号内文本。
/// 核心流程：按顶层（不在字符串、括号、花括号、尖括号里）的逗号切段，保留原文。
fn split_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut chars = args.chars();
    while let Some(ch) = chars.next() {
        if in_string {
            current.push(ch);
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' | '[' | '{' | '<' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth <= 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out.into_iter().filter(|segment| !segment.trim().is_empty()).collect()
}

/// 参数：`text` 为一段文本。
/// 核心流程：成对扫描 `"..."`，按顺序返回其中全部字面量。
fn quoted_values(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

/// 参数：`text` 为一段文本。
/// 核心流程：跳过字符串字面量后找 `+`，出现即说明路径是拼接表达式。
fn has_concat(text: &str) -> bool {
    let mut in_string = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if in_string && ch == '\\' {
            chars.next();
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if ch == '+' && !in_string {
            return true;
        }
    }
    false
}


/// 参数：`rest` 为 `@` 之后的文本。
/// 核心流程：取到 `(`、空白或行尾为止，再剥掉 `get:` 这类 use-site target 与包名。
fn annotation_short_name(rest: &str) -> &str {
    let end = rest.find(|c: char| c == '(' || c.is_whitespace()).unwrap_or(rest.len());
    let name = rest[..end].rsplit(':').next().unwrap_or(&rest[..end]);
    name.rsplit('.').next().unwrap_or(name)
}

/// 参数：`text` 为从 `@` 开始的注解文本。
/// 核心流程：取 `(` 之后的内容；没有括号返回空串。
fn args_of(text: &str) -> &str {
    match text.find('(') {
        Some(open) => &text[open + 1..],
        None => "",
    }
}

/// 参数：`name` 为注解简名。
/// 核心流程：五个 JAX-RS 动词注解映射到枚举，其余返回 None。
fn jaxrs_method(name: &str) -> Option<HttpMethod> {
    Some(match name {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
        "PATCH" => HttpMethod::Patch,
        _ => return None,
    })
}

/// 参数：`line` 为一行源码。
/// 核心流程：出现独立单词 class/interface/enum/record 即视为类型声明。
fn is_type_line(line: &str) -> bool {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| matches!(word, "class" | "interface" | "enum" | "record"))
}

/// 参数：`line` 为一行源码（已去首尾空白）。
/// 核心流程：从 `@` 取到 `(`，中间去空白即注解名，返回（注解名，括号内文本）。
fn parse_annotation(line: &str) -> Option<(&str, &str)> {
    let at = line.find('@')?;
    let open = line[at..].find('(')? + at;
    Some((line[at + 1..open].trim(), &line[open + 1..]))
}

/// 参数：`text` 为任意一行片段。
/// 核心流程：定位首个 `"`，再找其后的 `"` 收尾；缺任一端返回 None。
pub(crate) fn first_quoted(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_string())
}

/// 参数：`line` 为已去空白、不以注解开头的单行文本。
/// 核心流程：取 `(` 之前的部分为 head，截出末尾标识符；head 为空或含 `@` 视为非声明行。
fn handler_name(line: &str) -> Option<String> {
    let head = line[..line.find('(')?].trim_end();
    if head.is_empty() || head.contains('@') { return None; }
    let start = head.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).map_or(0, |i| i + 1);
    head[start..].starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$').then(|| head[start..].to_string())
}

/// 参数：`line` 为一行 Kotlin 源码。
/// 核心流程：取 `fun` 之后的第一个标识符作方法名；没有 `fun` 返回 None。
fn kotlin_handler(line: &str) -> Option<String> {
    let at = line.find("fun ")?;
    let name: String = line[at + 4..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty() && line[at + 4..].contains('(')).then_some(name)
}

/// 参数：`line` 为方法声明行。
/// 核心流程：Kotlin 取 `fun` 之后的名字；否则交给 Java 的 `handler_name`。
fn declared_name(line: &str) -> Option<String> {
    kotlin_handler(line).or_else(|| handler_name(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(body: &str) -> String {
        format!("package demo;\n\n@RestController\npublic class C {{\n{body}\n}}\n")
    }

    #[test]
    fn array_paths_and_multiple_verbs_form_a_cartesian_product() {
        let text = src("    @RequestMapping(value = {\"/a\", \"/b\"}, method = {RequestMethod.GET, RequestMethod.POST})\n    public void both() {\n    }");
        let found: Vec<(HttpMethod, String)> = scan_file(&text).into_iter().map(|e| (e.http_method, e.path)).collect();
        assert_eq!(
            found,
            vec![
                (HttpMethod::Get, "/a".to_string()),
                (HttpMethod::Get, "/b".to_string()),
                (HttpMethod::Post, "/a".to_string()),
                (HttpMethod::Post, "/b".to_string()),
            ]
        );
    }

    #[test]
    fn every_array_path_of_a_get_mapping_is_produced() {
        let text = src("    @GetMapping({\"/a\", \"/b\"})\n    public void both() {\n    }");
        let endpoints = scan_file(&text);
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].path, "/a");
        assert_eq!(endpoints[1].path, "/b");
        assert!(endpoints.iter().all(|e| !e.unresolved));
    }

    #[test]
    fn constants_placeholders_and_concatenation_are_unresolved() {
        let text = src("    @GetMapping(ApiConst.USERS)\n    public void byConst() {\n    }\n\n    @GetMapping(\"${api.base}/users\")\n    public void byPlaceholder() {\n    }\n\n    @GetMapping(ApiConst.BASE + \"/x\")\n    public void byConcat() {\n    }");
        let endpoints = scan_file(&text);
        assert_eq!(endpoints.len(), 3);
        assert!(endpoints.iter().all(|e| e.unresolved), "{endpoints:?}");
        assert_eq!(endpoints[0].path, "ApiConst.USERS");
        assert_eq!(endpoints[1].path, "${api.base}/users");
    }

    #[test]
    fn non_path_attributes_never_become_paths() {
        let text = src("    @RequestMapping(value = \"/v1/chat\", method = RequestMethod.POST, headers = \"X-API-KEY=MODEL_REPORTER\")\n    public String chat() {\n        return null;\n    }\n\n    @Headers(\"X-API-KEY=MODEL_REPORTER\")\n    @GetMapping(\"/v1/models\")\n    public String models() {\n        return null;\n    }");
        let endpoints = scan_file(&text);
        let paths: Vec<&str> = endpoints.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["/v1/chat", "/v1/models"], "headers/其他属性里的引号串不算路径");
        assert!(endpoints.iter().all(|e| !e.path.contains("X-API-KEY")));
    }

    #[test]
    fn feign_client_interfaces_are_skipped() {
        let feign = "@FeignClient(name = \"model-reporter\")\npublic interface ModelClient {\n    @GetMapping(\"/v1/models\")\n    String models();\n}\n";
        assert!(scan_file(feign).is_empty(), "只有 @FeignClient 的接口不算本项目端点");
        let controller = "@FeignClient(name = \"model-reporter\")\n@RestController\npublic class ModelController {\n    @GetMapping(\"/v1/models\")\n    public String models() {\n        return null;\n    }\n}\n";
        assert_eq!(scan_file(controller).len(), 1, "同时有 @RestController 的照常解析");
    }

    #[test]
    fn literal_paths_stay_resolved() {
        let text = src("    @GetMapping({\"/users/{id}\", \"/users\"})\n    public void get(int id) {\n    }");
        let endpoints = scan_file(&text);
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.iter().all(|e| !e.unresolved));
        assert_eq!(endpoints[0].path, "/users/{id}");
    }

    #[test]
    fn jaxrs_and_kotlin_paths_carry_the_same_flag() {
        let jaxrs = "@Path(\"/api\")\npublic class R {\n    @GET\n    @Path(Paths.USERS)\n    public User get(Long id) {\n        return null;\n    }\n}\n";
        let endpoints = scan_jaxrs(jaxrs);
        assert_eq!(endpoints.len(), 1);
        assert!(endpoints[0].unresolved);

        let kotlin = "@RestController\n@RequestMapping([\"/k\", \"/j\"])\nclass K {\n    @GetMapping([\"/items\", \"/goods\"])\n    fun list(): List<Item> = TODO()\n}\n";
        let endpoints = scan_kotlin(kotlin);
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.iter().all(|e| !e.unresolved && e.handler == "list"));
    }
    #[test]
    fn fastapi_line_numbers_point_at_the_decorator() {
        let single = "from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.get(\"/a\")\ndef a():\n    return 1\n";
        assert_eq!(scan_fastapi(single)[0].line, 5, "单行装饰器：注解行");

        let multi = "from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.get(\n    \"/b\",\n)\ndef b():\n    return 2\n";
        assert_eq!(scan_fastapi(multi)[0].line, 5, "跨行装饰器：仍是 @ 所在行");

        let stacked = "from fastapi import FastAPI\n\napp = FastAPI()\n\n@limiter.limit(\"5/minute\")\n@app.get(\"/c\")\ndef c():\n    return 3\n";
        let endpoints = scan_fastapi(stacked);
        assert_eq!(endpoints.len(), 1, "限流装饰器不产生路由");
        assert_eq!(endpoints[0].line, 6, "堆叠时取路由装饰器那一行");
    }

    #[test]
    fn aiohttp_style_files_are_gated_out() {
        let text = "from aiohttp import web\n\nroutes = web.RouteTableDef()\n\n@routes.get(\"/items\")\nasync def items(request):\n    return web.json_response([])\n\n@self.routes.post(\"/x\")\nasync def x(request):\n    return None\n";
        assert!(scan_fastapi(text).is_empty(), "没有 from fastapi 就整份跳过");
        assert!(!looks_like_fastapi(text), "预筛也认门禁");
    }

    #[test]
    fn indented_decorators_inside_fixtures_are_skipped() {
        let text = "from fastapi import FastAPI\n\napp = FastAPI()\n\nclass Fixture:\n    @app.get(\"/fake\")\n    def fake(self):\n        return 1\n\n@app.get(\"/real\")\ndef real():\n    return 2\n";
        let endpoints = scan_fastapi(text);
        assert_eq!(endpoints.len(), 1, "只认列 0 的装饰器");
        assert_eq!(endpoints[0].path, "/real");
    }

    #[test]
    fn generic_arguments_with_brackets_do_not_truncate_the_decorator() {
        let text = "from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.get(\"/items\", response_model=StandardResponse[list[Item]])\ndef items():\n    return []\n";
        let endpoints = scan_fastapi(text);
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].path, "/items");
        assert_eq!(endpoints[0].handler, "items");
    }

    #[test]
    fn head_and_options_map_to_their_own_methods() {
        let text = "from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.head(\"/ping\")\ndef head_ping():\n    return None\n\n@app.options(\"/ping\")\ndef options_ping():\n    return None\n";
        let endpoints = scan_fastapi(text);
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].http_method, HttpMethod::Head);
        assert_eq!(endpoints[1].http_method, HttpMethod::Options);
    }

    #[test]
    fn bom_files_and_websockets_are_handled() {
        let text = "\u{feff}from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.get(\"/bom\")\ndef bom():\n    return 1\n\n@app.websocket(\"/ws\")\nasync def ws(socket):\n    await socket.accept()\n";
        let endpoints = scan_fastapi(text);
        assert_eq!(endpoints.len(), 1, "BOM 不影响解析；websocket 本期跳过");
        assert_eq!(endpoints[0].path, "/bom");
        assert_eq!(websocket_decorators(text), 1, "跳过的 websocket 要能计数");
    }

}
