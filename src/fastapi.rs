//! FastAPI 跨文件前缀图（P2）：把 `include_router` 的链条拼成完整前缀。
//!
//! 纯函数：输入所有 `.py` 的（路径，文本），输出「文件 → router 变量 → 完整前缀」。
//! 图在建索引时构建并随索引落盘，视图/弹窗只读路由，不重复解析。

use std::collections::{BTreeMap, BTreeSet};

/// 前缀图：`文件路径 → router 变量名 → 完整前缀`（已含该 router 自身的 `APIRouter(prefix=)`）。
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrefixGraph {
    pub files: BTreeMap<String, BTreeMap<String, String>>,
}

impl PrefixGraph {
    /// 参数：`file` 为文件路径，`router` 为装饰器的接收者变量名。
    /// 核心流程：查图；查不到返回 None（调用方据此标 unresolved）。
    pub fn prefix_of(&self, file: &str, router: &str) -> Option<String> {
        self.files.get(file)?.get(router).cloned()
    }
}

/// 单个 .py 文件里与路由相关的声明。
#[derive(Default)]
struct Module {
    /// `var = APIRouter(prefix=...)` 的自身前缀（无 prefix 时为空串）。
    routers: BTreeMap<String, String>,
    /// `var = FastAPI(...)` 的变量名。
    apps: Vec<String>,
    /// 导入别名 →（模块名，原始符号名）。
    imports: BTreeMap<String, (String, String)>,
    /// `include_router(<ident>, prefix=...)`：父变量、子标识符、前缀。
    includes: Vec<(String, String, String)>,
}

/// 参数：`files` 为所有 `.py` 的（路径，文本）。
/// 核心流程：逐文件解析声明 → 连边（本文件变量或导入别名，含 re-export 跟随）→
/// 从 `FastAPI(` 的应用做 DFS（深度 ≤6、visited 去重防环）→ 累加前缀写入图。
pub fn build(files: &[(String, String)]) -> PrefixGraph {
    let mut modules: BTreeMap<String, Module> = BTreeMap::new();
    for (path, text) in files {
        modules.insert(path.clone(), parse_module(text));
    }
    let mut graph = PrefixGraph::default();
    let mut queue: Vec<(String, String, String)> = Vec::new();
    for (path, module) in &modules {
        for app in &module.apps {
            queue.push((path.clone(), app.clone(), String::new()));
        }
    }
    let mut visited: BTreeSet<(String, String)> = BTreeSet::new();
    let mut rounds = 0usize;
    while let Some((file, router, inherited)) = queue.pop() {
        rounds += 1;
        if rounds > 4096 || !visited.insert((file.clone(), router.clone())) {
            continue;
        }
        let Some(module) = modules.get(&file) else { continue };
        let own = module.routers.get(&router).cloned().unwrap_or_default();
        let full = join(&inherited, &own);
        graph.files.entry(file.clone()).or_default().insert(router.clone(), full.clone());
        if depth_of(&full) > 6 {
            continue;
        }
        for (parent, child, prefix) in &module.includes {
            if parent != &router {
                continue;
            }
            for (child_file, child_router) in resolve_symbol(&modules, &file, child, 0) {
                queue.push((child_file, child_router, join(&full, prefix)));
            }
        }
    }
    graph
}

/// 参数：`text` 为文件全文。
/// 核心流程：逐行抓 `APIRouter(`/`FastAPI(`/`from … import …`/`include_router(`；
/// `include_router` 全单行，取第一个参数标识符与 `prefix=` 的字符串字面量。
fn parse_module(text: &str) -> Module {
    let mut module = Module::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("from ") {
            if let Some((name, alias)) = parse_import(rest) {
                module.imports.insert(alias, name);
                continue;
            }
        }
        if line.contains("APIRouter(") {
            if let (Some(var), Some(args)) = (assigned_var(line), args_of(line, "APIRouter(")) {
                module.routers.insert(var, first_string(args).unwrap_or_default());
            }
            continue;
        }
        if line.contains("FastAPI(") {
            if let Some(var) = assigned_var(line) {
                module.apps.push(var);
            }
            continue;
        }
        if let Some(at) = line.find("include_router(") {
            let receiver = line[..at].trim_end().trim_end_matches('.').trim().to_string();
            let args = &line[at + "include_router(".len()..];
            let Some(child) = args.split([',', ')']).next().map(str::trim).filter(|c| !c.is_empty()) else { continue };
            if !child.chars().all(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let prefix = first_string(args).unwrap_or_default();
            module.includes.push((receiver, child.to_string(), prefix));
        }
    }
    module
}

/// 参数：`rest` 为 `from ` 之后、`import` 之前的整段。
/// 核心流程：拆出模块名与第一个符号（`import router` / `import router as users_router`）；
/// 多个符号只取第一个，`*` 与无 `import` 的行忽略。
fn parse_import(rest: &str) -> Option<((String, String), String)> {
    let (module, names) = rest.split_once(" import ")?;
    let module = module.trim();
    if module.is_empty() {
        return None;
    }
    let first = names.split(',').next()?.trim();
    if first == "*" || first.is_empty() {
        return None;
    }
    let (symbol, alias) = match first.split_once(" as ") {
        Some((symbol, alias)) => (symbol.trim(), alias.trim()),
        None => (first, first),
    };
    if symbol.is_empty() || alias.is_empty() {
        return None;
    }
    Some(((module.to_string(), symbol.to_string()), alias.to_string()))
}

/// 参数：`line` 为一行源码。
/// 核心流程：取 `=` 左边最后一个标识符作为被赋值变量（`router = APIRouter(...)`）。
fn assigned_var(line: &str) -> Option<String> {
    let left = line.split('=').next()?.trim();
    let var = left.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).find(|part| !part.is_empty())?;
    Some(var.to_string())
}

/// 参数：`line` 为一源码行，`needle` 为 `APIRouter(`/`FastAPI(` 之类。
/// 核心流程：取 needle 之后的括号内文本（同一行）。
fn args_of<'a>(line: &'a str, needle: &str) -> Option<&'a str> {
    let at = line.find(needle)? + needle.len();
    Some(&line[at..])
}

/// 参数：`text` 为一段源码。
/// 核心流程：取第一个字符串字面量（单双引号都支持）。
pub(crate) fn first_string(text: &str) -> Option<String> {
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

/// 参数：`modules` 为全部文件的声明，`file` 为当前文件，`symbol` 为被 include 的标识符，
/// `depth` 为 re-export 跟随层数。
/// 核心流程：本文件有 `APIRouter` 定义即命中；否则按导入别名解析到目标文件，
/// 目标文件只是 re-export 时继续跟随（≤4 层）。
fn resolve_symbol(
    modules: &BTreeMap<String, Module>,
    file: &str,
    symbol: &str,
    depth: usize,
) -> Vec<(String, String)> {
    if depth > 4 {
        return Vec::new();
    }
    let Some(module) = modules.get(file) else { return Vec::new() };
    if module.routers.contains_key(symbol) {
        return vec![(file.to_string(), symbol.to_string())];
    }
    let Some((module_name, original)) = module.imports.get(symbol) else { return Vec::new() };
    let mut out = Vec::new();
    for candidate in resolve_module(modules, file, module_name) {
        out.extend(resolve_symbol(modules, &candidate, original, depth + 1));
    }
    out
}

/// 参数：`modules` 为全部文件，`file` 为导入方，`module_name` 为 `from X import …` 的 X。
/// 核心流程：相对导入按当前目录逐级上溯，绝对导入按项目内后缀匹配；`x.py` 与 `x/__init__.py` 都试。
fn resolve_module(modules: &BTreeMap<String, Module>, file: &str, module_name: &str) -> Vec<String> {
    let (dots, rest) = {
        let dots = module_name.chars().take_while(|c| *c == '.').count();
        (dots, module_name[dots..].trim_start_matches('.'))
    };
    let base = file.rsplit_once('/').map_or(String::new(), |(dir, _)| dir.to_string());
    let mut out = Vec::new();
    if dots > 0 {
        let mut dir = base.clone();
        for _ in 1..dots {
            dir = dir.rsplit_once('/').map_or(String::new(), |(parent, _)| parent.to_string());
        }
        let tail = rest.replace('.', "/");
        let stem = if tail.is_empty() { dir.clone() } else { format!("{dir}/{tail}") };
        for candidate in [format!("{stem}.py"), format!("{stem}/__init__.py")] {
            if modules.contains_key(&candidate) {
                out.push(candidate);
            }
        }
    } else {
        let suffix = format!("{}/", rest.replace('.', "/"));
        let exact = format!("{}.py", rest.replace('.', "/"));
        let init = format!("{}/__init__.py", rest.replace('.', "/"));
        for path in modules.keys() {
            if path.ends_with(&exact) || path.ends_with(&init) || path.contains(&suffix) && path.ends_with("/__init__.py") {
                out.push(path.clone());
            }
        }
    }
    out
}

/// 参数：`parts` 为要拼接的前缀片段。
/// 核心流程：去掉首尾多余斜杠、合并重复斜杠；全空时为空串。
fn join(left: &str, right: &str) -> String {
    let mut out = String::new();
    for part in [left, right] {
        for segment in part.split('/').filter(|segment| !segment.is_empty()) {
            out.push('/');
            out.push_str(segment);
        }
    }
    out
}

/// 参数：`prefix` 为已拼好的前缀。
/// 核心流程：用段数近似 DFS 深度（配合 visited 防环，够用且不需要额外传参）。
fn depth_of(prefix: &str) -> usize {
    prefix.split('/').filter(|segment| !segment.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{scan_fastapi_with, HttpMethod};

    fn files(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(path, text)| ((*path).to_string(), (*text).to_string())).collect()
    }

    fn endpoints(files: &[(String, String)], target: &str) -> Vec<crate::scan::Endpoint> {
        let graph = build(files);
        let text = files.iter().find(|(path, _)| path == target).map(|(_, text)| text.clone()).unwrap_or_default();
        scan_fastapi_with(&text, Some(&|router: &str| graph.prefix_of(target, router)))
    }

    #[test]
    fn alias_differs_from_the_file_name() {
        let input = files(&[
            (
                "pkg/api/users.py",
                "from fastapi import APIRouter\n\nrouter = APIRouter(prefix=\"/users\")\n\n@router.get(\"/{id}\")\ndef get_user(id: int):\n    return None\n",
            ),
            (
                "pkg/api/routes.py",
                "from fastapi import APIRouter\nfrom .users import router as users_router\n\nrouter = APIRouter()\nrouter.include_router(users_router, prefix=\"/api\")\n",
            ),
            (
                "pkg/app.py",
                "from fastapi import FastAPI\nfrom .api.routes import router\n\napp = FastAPI()\napp.include_router(router)\n",
            ),
        ]);
        let found = endpoints(&input, "pkg/api/users.py");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/api/users/{id}", "别名 + 链上的 prefix 都要拼上");
        assert!(!found[0].unresolved);
    }

    #[test]
    fn re_export_shells_are_followed() {
        let input = files(&[
            (
                "pkg/api/users.py",
                "from fastapi import APIRouter\n\nrouter = APIRouter(prefix=\"/users\")\n\n@router.get(\"\")\ndef list_users():\n    return []\n",
            ),
            ("pkg/api/__init__.py", "from .users import router\n"),
            (
                "pkg/app.py",
                "from fastapi import FastAPI\nfrom .api import router\n\napp = FastAPI()\napp.include_router(router, prefix=\"/v1\")\n",
            ),
        ]);
        let found = endpoints(&input, "pkg/api/users.py");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/v1/users", "空路径只留前缀，且穿过 __init__ 空壳");
        assert!(!found[0].unresolved);
    }

    #[test]
    fn five_hop_chain_accumulates_every_prefix() {
        let mut input: Vec<(String, String)> = vec![(
            "pkg/app.py".into(),
            "from fastapi import FastAPI\nfrom .a2 import router\n\napp = FastAPI()\napp.include_router(router, prefix=\"/1\")\n".into(),
        )];
        for level in 2..=4 {
            input.push((
                format!("pkg/a{level}.py"),
                format!(
                    "from fastapi import APIRouter\nfrom .a{} import router as next_router\n\nrouter = APIRouter()\nrouter.include_router(next_router, prefix=\"/{level}\")\n",
                    level + 1
                ),
            ));
        }
        input.push((
            "pkg/a5.py".into(),
            "from fastapi import APIRouter\n\nrouter = APIRouter()\n\n@router.get(\"/x\")\ndef x():\n    return None\n".into(),
        ));
        let found = endpoints(&input, "pkg/a5.py");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/1/2/3/4/x", "5 跳链的前缀全部累加");
        assert!(!found[0].unresolved);
    }

    #[test]
    fn cycles_do_not_hang() {
        let input = files(&[
            ("pkg/a.py", "from fastapi import FastAPI\nfrom .b import router as b_router\n\napp = FastAPI()\napp.include_router(b_router, prefix=\"/a\")\n"),
            ("pkg/b.py", "from fastapi import APIRouter\nfrom .a import app as back\n\nrouter = APIRouter()\nrouter.include_router(back, prefix=\"/b\")\n"),
        ]);
        let graph = build(&input);
        assert!(graph.prefix_of("pkg/b.py", "router").is_some(), "循环 include 也要收敛出前缀");
    }

    #[test]
    fn include_without_prefix_keeps_the_hardcoded_path() {
        let input = files(&[
            (
                "pkg/api/items.py",
                "from fastapi import APIRouter\n\nrouter = APIRouter()\n\n@router.post(\"/v1/items\")\ndef create():\n    return None\n",
            ),
            (
                "pkg/app.py",
                "from fastapi import FastAPI\nfrom .api.items import router\n\napp = FastAPI()\napp.include_router(router)\n",
            ),
        ]);
        let found = endpoints(&input, "pkg/api/items.py");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].http_method, HttpMethod::Post);
        assert_eq!(found[0].path, "/v1/items", "无前缀的 include 不改变硬编码路径");
        assert!(!found[0].unresolved);
    }

    #[test]
    fn unreachable_routers_stay_relative_and_unresolved() {
        let input = files(&[(
            "pkg/orphan.py",
            "from fastapi import APIRouter\n\nrouter = APIRouter()\n\n@router.get(\"/alone\")\ndef alone():\n    return None\n",
        )]);
        let found = endpoints(&input, "pkg/orphan.py");
        assert_eq!(found[0].path, "/alone");
        assert!(found[0].unresolved, "没挂到 FastAPI 上的 router 保持相对路径并标未解析");
    }
}
