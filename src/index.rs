//! 路由索引（US-E1-02 / US-E1-03）：把一批「路径 + 文本」算成可序列化的路由表。
//!
//! 路径一律先经 `url::join_url` 归一化（前导 `/`、无重复斜杠、无尾斜杠），
//! 含空白的非法路径在入索引前丢弃，避免污染 `endpoints` 的 1 万条上限。

use serde::{Deserialize, Serialize};

use crate::scan::{self, HttpMethod};
use crate::url;

/// 一条路由：HTTP 方法、归一化后的完整路径、处理方法名、注解行（1 基）与来源文件。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
    pub handler: String,
    pub line: u32,
    pub file: String,
    /// 路径没能静态解析（原文保留），复制 URL 时据此提示。
    pub unresolved: bool,
    /// `spring` 或 `jaxrs`，endpoints 的 framework 直接用它。
    pub framework: String,
    /// 前端路由模块（v7 `route(path, module, ...)`）的跳转位置；有它时跳转优先用它。
    #[serde(default)]
    pub jump: Option<Jump>,
}

/// 第二跳位置：路由声明指向的模块文件（行号 1 基，打开时再转 0 基）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Jump {
    pub file: String,
    pub line: u32,
}

/// 全项目路由索引，整体可按 JSON 存进 `storage`。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RouteIndex {
    pub routes: Vec<Route>,
}

impl RouteIndex {
    /// 参数：`files` 为（文件路径，文件文本）列表，路径原样记录进 `Route::file`。
    /// 核心流程：逐文件扫描映射 → 取该端点上方最近的类级 `@RequestMapping` 当前缀 →
    /// `url::join_url` 拼成完整路径 → 丢弃非法或重复的路由（按文件+方法+路径+行去重）。
    pub fn from_files(files: Vec<(String, String)>) -> Self {
        let mut routes: Vec<Route> = Vec::new();
        for (file, text) in files {
            for route in routes_of(&file, &text) {
                if !routes.contains(&route) {
                    routes.push(route);
                }
            }
        }
        Self { routes }
    }

    /// 参数：`path` 为文件路径，`text` 为最新全文。
    /// 核心流程：先删这个文件的旧条目，再只重扫它；新条目插回原位置，其他文件的条目与顺序不动。
    pub fn update_file(&mut self, path: &str, text: &str) {
        let at = self.routes.iter().position(|route| route.file == path).unwrap_or(self.routes.len());
        self.routes.retain(|route| route.file != path);
        let fresh = routes_of(path, text);
        let at = at.min(self.routes.len());
        self.routes.splice(at..at, fresh);
    }

    /// 参数：`path` 为已删除（或改名）的文件路径。
    /// 核心流程：丢掉该文件的全部条目，返回删掉的条数。
    pub fn remove_file(&mut self, path: &str) -> usize {
        let before = self.routes.len();
        self.routes.retain(|route| route.file != path);
        before - self.routes.len()
    }

    /// 参数：`path` 为文件路径，`endpoints` 为外部扫好的端点（带前缀图的 Python 用）。
    /// 核心流程：与 `update_file` 相同，只是不再自己扫文件。
    pub fn update_file_scanned(&mut self, path: &str, text: &str, endpoints: Vec<scan::Endpoint>) {
        let at = self.routes.iter().position(|route| route.file == path).unwrap_or(self.routes.len());
        self.routes.retain(|route| route.file != path);
        let fresh = routes_from(path, text, endpoints);
        let at = at.min(self.routes.len());
        self.routes.splice(at..at, fresh);
    }

    /// 参数：`changes` 为「文件 → 最新全文」，只应包含新增或内容变了的文件。
    /// 核心流程：逐个 `update_file`，返回被重扫的文件数；未列出的文件一律不动。
    pub fn apply_changes(&mut self, changes: Vec<(String, String)>) -> usize {
        for (path, text) in &changes {
            self.update_file(path, text);
        }
        changes.len()
    }

    /// 参数：`self` 为待持久化的索引。
    /// 核心流程：`serde_json::to_string`；失败时给一句可读错误文本而不是 panic。
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("索引序列化失败：{e}"))
    }

    /// 参数：`text` 为 `to_json` 写出的 JSON。
    /// 核心流程：`serde_json::from_str`；内容损坏时给一句可读错误文本。
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("索引反序列化失败：{e}"))
    }
}


/// 参数：`file` 为文件路径，`text` 为全文。
/// 核心流程：扫出端点 → 类级路径（Spring 优先，JAX-RS 兜底）→ 解析成功的路径归一化，
/// 未解析的保留原文 → 按「文件+方法+路径+行」去重。
fn routes_of(file: &str, text: &str) -> Vec<Route> {
    routes_from(file, text, scan::scan_any(file, text))
}

/// 参数：`file`/`text` 为来源，`endpoints` 为已扫好的端点（Python 会用前缀图增强路径）。
/// 核心流程：端点 → 类级路径 → 归一化路径 → `Route`，按「文件+方法+路径+行」去重。
fn routes_from(file: &str, text: &str, endpoints: Vec<scan::Endpoint>) -> Vec<Route> {
    let mut routes: Vec<Route> = Vec::new();
    for endpoint in endpoints {
        let base = if endpoint.react {
            String::new()
        } else {
            match class_path_above(text, endpoint.line) {
                base if !base.is_empty() => base,
                _ => scan::jaxrs_class_path(text),
            }
        };
        // 全部补前导 `/`：宿主会整批校验路径，缺斜杠会让整份 endpoints 作废。
        let path = if endpoint.unresolved {
            let raw = sanitize_unresolved(&endpoint.path);
            if raw.starts_with('/') { raw } else { format!("/{raw}") }
        } else {
            let joined = url::join_url("", &base, &endpoint.path);
            if !valid_path(&joined) {
                continue;
            }
            joined
        };
        let route = Route {
            method: verb(&endpoint.http_method).to_string(),
            path,
            handler: endpoint.handler,
            line: endpoint.line,
            file: file.to_string(),
            unresolved: endpoint.unresolved,
            framework: if endpoint.react {
                "react-router".to_string()
            } else if endpoint.fastapi {
                "fastapi".to_string()
            } else if endpoint.jaxrs {
                "jaxrs".to_string()
            } else {
                "spring".to_string()
            },
            jump: endpoint.module.as_deref().map(|module| Jump { file: resolve_module(file, module), line: 1 }),
        };
        if !routes.contains(&route) {
            routes.push(route);
        }
    }
    routes
}

/// 参数：`text` 为文件全文。
/// 核心流程：FNV-1a 64 位哈希，用来判断文件自上次索引后有没有变（不保存原文）。
pub fn signature(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// 参数：`text` 为文件全文，`line` 为该端点注解所在的 1 基行号。
/// 核心流程：从上一行往上找最近的含 `RequestMapping` 的行，取其括号内第一个双引号串；
/// 找不到返回空串，此时方法路径按绝对路径处理。
pub(crate) fn class_path_above(text: &str, line: u32) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = (line as usize).saturating_sub(1).min(lines.len());
    lines[..end]
        .iter()
        .rev()
        .find(|l| l.contains("RequestMapping"))
        .and_then(|l| scan::first_quoted(l))
        .unwrap_or_default()
}

/// 参数：`m` 为解析出的注解方法。
/// 核心流程：五个动词原样大写；未写明动词的 `@RequestMapping` 记作 `ANY`。
pub(crate) fn verb(m: &HttpMethod) -> &'static str {
    match m {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Head => "HEAD",
        HttpMethod::Options => "OPTIONS",
        HttpMethod::RequestMapping => "ANY",
    }
}

/// 参数：`path` 为拼装后的路径。
/// 核心流程：必须以 `/` 开头且不含任何空白字符，否则视为非法路径。
fn valid_path(path: &str) -> bool {
    path.starts_with('/') && !path.chars().any(char::is_whitespace)
}

/// 参数：`raw` 为未解析的路径原文（常量、三元表达式、函数调用等）。
/// 核心流程：去掉空白与控制字符并截到 8192 字节，保证宿主整批校验不被单条打回。
fn sanitize_unresolved(raw: &str) -> String {
    raw.chars().filter(|ch| !ch.is_whitespace() && !ch.is_control()).take(8192).collect()
}

/// 参数：`file` 为声明路由的文件，`module` 为 v7 `route()` 的模块参数。
/// 核心流程：以配置文件所在目录为基准拼接，展开 `.` 与 `..`，保留绝对路径。
fn resolve_module(file: &str, module: &str) -> String {
    let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir);
    let absolute = module.starts_with('/') || dir.starts_with('/');
    let segments = if module.starts_with('/') {
        module.split('/').collect::<Vec<_>>()
    } else {
        dir.split('/').chain(module.split('/')).collect()
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in segments {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute { format!("/{joined}") } else { joined }
}

/// 参数：`route` 为索引里的一条路由。
/// 核心流程：解析成功的路径一律以 `/` 开头；带 `${...}` 或不以 `/` 开头说明当时没解开。
pub fn is_unresolved(route: &Route) -> bool {
    route.unresolved
}

/// 参数：`route` 为索引里的一条路由。
/// 核心流程：未解析时给一句可读提示（带原文与补救办法），解析成功返回 None。
pub fn unresolved_hint(route: &Route) -> Option<String> {
    is_unresolved(route).then(|| {
        format!("路径未静态解析，已保留原文「{}」：请把常量补成字面量，或在配置里提供 {} 占位符的取值后再复制", route.path, route.path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller(class_path: &str, method_path: &str, handler: &str) -> String {
        format!(
            "package demo;\n\n@RestController\n@RequestMapping(\"{class_path}\")\npublic class C {{\n    @GetMapping(\"{method_path}\")\n    public String {handler}() {{\n        return null;\n    }}\n}}\n"
        )
    }

    #[test]
    fn merges_routes_across_files() {        let index = RouteIndex::from_files(vec![
            ("src/A.java".into(), controller("/api", "/users", "users")),
            ("src/B.java".into(), controller("/admin/", "/stats", "stats")),
        ]);
        let got: Vec<&str> = index.routes.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(got, ["/api/users", "/admin/stats"]);
        assert_eq!(index.routes[0].method, "GET");
        assert_eq!(index.routes[0].file, "src/A.java");
    }

    #[test]
    fn one_file_with_two_controllers_keeps_both_base_paths() {
        let text = format!(
            "{}\n{}",
            controller("/api", "/users", "users"),
            controller("/admin", "/stats", "stats")
        );
        let index = RouteIndex::from_files(vec![("src/C.java".into(), text)]);
        let got: Vec<&str> = index.routes.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(got, ["/api/users", "/admin/stats"]);
    }

    #[test]
    fn jaxrs_and_kotlin_files_are_indexed() {
        let jaxrs = "@Path(\"/api\")\npublic class UserResource {\n    @GET\n    @Path(\"/users/{id}\")\n    public User get(Long id) {\n        return null;\n    }\n}\n";
        let kotlin = "@RestController\n@RequestMapping(\"/k\")\nclass KController {\n    @PostMapping(\"/items\")\n    fun create(body: Item): Item { return body }\n}\n";
        let index = RouteIndex::from_files(vec![
            ("src/UserResource.java".into(), jaxrs.to_string()),
            ("src/KController.kt".into(), kotlin.to_string()),
        ]);
        let got: Vec<(&str, &str)> = index.routes.iter().map(|route| (route.method.as_str(), route.path.as_str())).collect();
        assert_eq!(got, vec![("GET", "/api/users/{id}"), ("POST", "/k/items")]);
    }

    #[test]
    fn feign_clients_do_not_enter_the_index_but_controllers_do() {
        let feign = "@FeignClient(name = \"model-reporter\")\npublic interface ModelClient {\n    @Headers(\"X-API-KEY=MODEL_REPORTER\")\n    @GetMapping(\"/v1/models\")\n    String models();\n}\n";
        let controller = controller("/api", "/users", "users");
        let index = RouteIndex::from_files(vec![("src/ModelClient.java".into(), feign.into()), ("src/UserController.java".into(), controller)]);
        let got: Vec<&str> = index.routes.iter().map(|route| route.path.as_str()).collect();
        assert_eq!(got, ["/api/users"], "Feign 客户端被跳过，RestController 照旧");
    }

    #[test]
    fn fastapi_files_are_indexed_with_their_own_framework() {
        let text = "from fastapi import APIRouter\n\nrouter = APIRouter(prefix=\"/api\")\n\n@router.get(\"/items\")\ndef items():\n    return []\n";
        let index = RouteIndex::from_files(vec![("app/routers/items.py".into(), text.into())]);
        assert_eq!(index.routes.len(), 1);
        assert_eq!(index.routes[0].path, "/api/items");
        assert_eq!(index.routes[0].framework, "fastapi");
    }

    #[test]
    fn empty_input_is_an_empty_index() {
        let index = RouteIndex::from_files(Vec::new());
        assert!(index.routes.is_empty());
        assert_eq!(index, RouteIndex::default());
    }

    #[test]
    fn json_round_trip() {
        let index = RouteIndex::from_files(vec![("src/A.java".into(), controller("/api", "/users/{id}", "get"))]);
        let json = index.to_json().unwrap();
        assert_eq!(RouteIndex::from_json(&json), Ok(index));
        assert!(RouteIndex::from_json("{oops").is_err());
    }

    #[test]
    fn unresolved_paths_survive_the_index_and_carry_a_hint() {
        let text = "@RestController\npublic class U {\n    @GetMapping(ApiConst.USERS)\n    public void byConst() {\n    }\n\n    @GetMapping(\"${api.base}/users\")\n    public void byPlaceholder() {\n    }\n}\n";
        let index = RouteIndex::from_files(vec![("src/U.java".into(), text.into())]);
        assert_eq!(index.routes.len(), 2, "未解析也要留在索引里");
        assert!(index.routes.iter().all(is_unresolved));
        assert_eq!(index.routes[0].path, "/ApiConst.USERS", "未解析路径也补前导斜杠");
        let hint = unresolved_hint(&index.routes[1]).unwrap_or_default();
        assert!(hint.contains("${api.base}/users"), "{hint}");
        assert!(index.routes.iter().all(|route| route.path.starts_with('/')), "宿主会整批校验，必须都有前导斜杠");
        assert!(unresolved_hint(&RouteIndex::from_files(vec![("src/A.java".into(), controller("/api", "/x", "x"))]).routes[0]).is_none());
    }

    #[test]
    fn array_paths_and_verbs_become_four_indexed_routes() {
        let text = "@RestController\n@RequestMapping(\"/api\")\npublic class Both {\n    @RequestMapping(value = {\"/a\", \"/b\"}, method = {RequestMethod.GET, RequestMethod.POST})\n    public void both() {\n    }\n}\n";
        let index = RouteIndex::from_files(vec![("src/Both.java".into(), text.into())]);
        let got: Vec<(&str, &str)> = index.routes.iter().map(|route| (route.method.as_str(), route.path.as_str())).collect();
        assert_eq!(got, vec![("GET", "/api/a"), ("GET", "/api/b"), ("POST", "/api/a"), ("POST", "/api/b")]);
    }

    #[test]
    fn updating_one_file_keeps_the_others() {
        let mut index = RouteIndex::from_files(vec![
            ("src/A.java".into(), controller("/api", "/users", "users")),
            ("src/B.java".into(), controller("/admin", "/stats", "stats")),
        ]);
        let before = index.routes.iter().map(|route| route.path.clone()).collect::<Vec<_>>();
        index.update_file("src/A.java", &controller("/api", "/members", "members"));
        let after: Vec<&str> = index.routes.iter().map(|route| route.path.as_str()).collect();
        assert_eq!(after, ["/api/members", "/admin/stats"]);
        assert_eq!(index.routes[0].file, "src/A.java", "重扫的条目留在原位置");
        assert_eq!(before.len(), index.routes.len(), "条数不变，只换了这一条");
    }

    #[test]
    fn removing_a_file_drops_only_its_routes() {
        let mut index = RouteIndex::from_files(vec![
            ("src/A.java".into(), controller("/api", "/users", "users")),
            ("src/B.java".into(), controller("/admin", "/stats", "stats")),
        ]);
        assert_eq!(index.remove_file("src/A.java"), 1);
        assert_eq!(index.routes.len(), 1);
        assert_eq!(index.routes[0].file, "src/B.java");
        assert_eq!(index.remove_file("src/A.java"), 0, "再删一次是 0 条");
    }

    #[test]
    fn apply_changes_rescans_only_the_listed_files() {
        let mut index = RouteIndex::from_files(vec![
            ("src/A.java".into(), controller("/api", "/users", "users")),
            ("src/B.java".into(), controller("/admin", "/stats", "stats")),
        ]);
        let rescanned = index.apply_changes(vec![("src/B.java".into(), controller("/admin", "/health", "health"))]);
        assert_eq!(rescanned, 1);
        let got: Vec<&str> = index.routes.iter().map(|route| route.path.as_str()).collect();
        assert_eq!(got, ["/api/users", "/admin/health"]);
        assert_eq!(index.apply_changes(Vec::new()), 0);
    }

    #[test]
    fn signatures_change_with_the_text() {
        assert_eq!(signature("class A {}"), signature("class A {}"));
        assert_ne!(signature("class A {}"), signature("class B {}"));
    }

    #[test]
    fn duplicate_routes_are_dropped() {
        let text = controller("/api", "/x", "x");
        let index = RouteIndex::from_files(vec![("a.java".into(), text.clone()), ("a.java".into(), text)]);
        assert_eq!(index.routes.len(), 1, "同一文件的同一条路由只留一份");
    }

    #[test]
    fn v7_module_paths_resolve_relative_to_the_config_file() {
        let text = "import { route } from \"@react-router/dev/routes\";\nexport default [route(\"clipped\", \"./routes/clipped/index.tsx\"), route(\"shared\", \"../shared/page.tsx\")];\n";
        let index = RouteIndex::from_files(vec![("/project/app/routes.ts".into(), text.into())]);
        let files: Vec<&str> = index.routes.iter().map(|route| route.jump.as_ref().unwrap().file.as_str()).collect();
        assert_eq!(files, ["/project/app/routes/clipped/index.tsx", "/project/shared/page.tsx"]);
    }

    #[test]
    fn unresolved_paths_are_sanitized_for_the_host() {
        let text = "export const App = () => (\n  <Route path={cond ? '/a' : '/b'} element={<A />} />\n);\n";
        let index = RouteIndex::from_files(vec![("src/App.tsx".into(), text.into())]);
        assert_eq!(index.routes.len(), 1);
        let route = &index.routes[0];
        assert!(route.unresolved);
        assert!(valid_path(&route.path), "宿主整批校验不接受空白：{}", route.path);
    }
}
