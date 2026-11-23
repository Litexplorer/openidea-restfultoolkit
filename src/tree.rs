//! 路由树（US-E2-01）：把路由索引摊平成「模块 → 控制器 → 方法」三级节点。
//!
//! 宿主只认扁平 `list<tree-item>`，且 `parent` 只能指向更早的项，所以这里是
//! 纯函数：先按（模块，文件，行，方法，路径）稳定排序，再顺序压平。

use crate::index::{Route, RouteIndex};

/// 单次渲染的节点上限（US-E2-01：超出时截断并在状态行说明）。
pub const MAX_NODES: usize = 5000;

/// 树节点：key 唯一，`parent` 是更早节点的 key。
///
/// 方法节点没有独立图标：宿主的图标集（`general/*`、`diff/*` 等）里没有 HTTP 动词，
/// 未知名字会被画成通用文件图标，所以动词改用标签前缀 `[GET]` 呈现。
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub key: String,
    pub parent: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub expandable: bool,
}

/// 根节点 key。
pub const ROOT: &str = "root";

/// 空索引时根节点的提示（英文原文，渲染时按语言包本地化）。
pub const EMPTY_HINT: &str = "No Spring/JAX-RS routes found: run Restful Toolkit: Rebuild Route Index, and make sure the project has @RestController/@RequestMapping";

/// 参数：`index` 为路由索引。
/// 核心流程：先统计每个模块的服务数并稳定排序（模块、文件、行、方法、路径）→
/// 首节点是「Found N services」根 → 遇到新模块/新文件各压一个父节点 → 再压方法节点。
pub fn flatten(index: &RouteIndex) -> Vec<Node> {
    let mut routes: Vec<&Route> = index.routes.iter().collect();
    let files: Vec<&str> = routes.iter().map(|route| route.file.as_str()).collect();
    let subs = subprojects(&files);
    routes.sort_by(|a, b| {
        module_of_with(&a.file, &subs)
            .cmp(&module_of_with(&b.file, &subs))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.method.cmp(&b.method))
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut nodes = Vec::with_capacity(routes.len() + 8);
    // 空索引只给一个根节点，并把「为什么是空的 + 下一步做什么」写在 detail 里。
    nodes.push(if routes.is_empty() {
        Node {
            key: ROOT.into(),
            parent: None,
            label: "Restful Routes: 0 routes".to_string(),
            detail: Some(EMPTY_HINT.to_string()),
            expandable: false,
        }
    } else {
        Node {
            key: ROOT.into(),
            parent: None,
            label: format!("Found {} services", routes.len()),
            detail: None,
            expandable: true,
        }
    });
    let mut module = String::new();
    let mut file = String::new();
    for route in routes {
        let route_module = module_of_with(&route.file, &subs);
        if route_module != module {
            let count = index.routes.iter().filter(|r| module_of_with(&r.file, &subs) == route_module).count();
            nodes.push(Node {
                key: module_key(&route_module),
                parent: Some(ROOT.into()),
                label: route_module.clone(),
                detail: Some(format!("{count} services")),
                expandable: true,
            });
            module = route_module;
            file.clear();
        }
        if route.file != file {
            nodes.push(Node {
                key: class_key(&route.file),
                parent: Some(module_key(&module)),
                label: class_name(&route.file),
                detail: Some(route.file.clone()),
                expandable: true,
            });
            file = route.file.clone();
        }
        nodes.push(Node {
            key: route_key(route),
            parent: Some(class_key(&route.file)),
            label: format!("[{}] {}", route.method, route.handler),
            detail: Some(format!("{} {}", route.method, route.path)),
            expandable: false,
        });
    }
    nodes
}

/// 参数：`nodes` 为 `flatten` 的结果。
/// 核心流程：去掉模块节点，把原来的控制器节点挂回根（`groupBy = controller`），其余不动。
pub fn group_by_controller(nodes: Vec<Node>) -> Vec<Node> {
    nodes
        .into_iter()
        .filter_map(|mut node| {
            if node.key.starts_with("m|") {
                return None;
            }
            if node.parent.as_deref().is_some_and(|parent| parent.starts_with("m|")) {
                node.parent = Some(ROOT.to_string());
            }
            Some(node)
        })
        .collect()
}

/// 参数：`files` 为索引里出现的文件路径。
/// 核心流程：求所有文件所在目录的公共前缀，收集其下一级目录名（去重排序）；
/// 有 2 个及以上不同名字时认为项目根下有多个子项目，返回这些名字。
pub fn subprojects(files: &[&str]) -> Vec<String> {
    let dirs: Vec<Vec<&str>> = files
        .iter()
        .map(|file| file.rsplit_once('/').map_or("", |(dir, _)| dir).split('/').collect())
        .collect();
    let Some(first) = dirs.first() else { return Vec::new() };
    let mut common = first.len();
    for dir in &dirs {
        let shared = dir.iter().zip(first.iter()).take_while(|(a, b)| a == b).count();
        common = common.min(shared);
    }
    let names: std::collections::BTreeSet<&str> =
        dirs.iter().filter_map(|dir| dir.get(common).copied()).filter(|name| !name.is_empty()).collect();
    if names.len() >= 2 {
        names.into_iter().map(str::to_string).collect()
    } else {
        Vec::new()
    }
}

/// 参数：`file` 为文件路径，`subs` 为 `subprojects` 的结果。
/// 核心流程：路径里含某个子项目目录名时返回它，否则 None。
pub fn subproject_of(file: &str, subs: &[String]) -> Option<String> {
    subs.iter().find(|sub| file.contains(&format!("/{sub}/"))).cloned()
}

/// 参数：`file` 为文件路径，`subs` 为 `subprojects` 的结果。
/// 核心流程：有子项目层时模块名取子项目名，否则退回 `module_of`。
pub fn module_of_with(file: &str, subs: &[String]) -> String {
    subproject_of(file, subs).unwrap_or_else(|| module_of(file))
}

/// 参数：`file` 为文件路径（绝对或项目内相对）。
/// 核心流程：取 `/src/main/` 之前那段路径的最后一段作模块名；没有该前缀记 `default`。
pub fn module_of(file: &str) -> String {
    // Python：用所在目录名分组（app/routers/users.py → routers）。
    if file.ends_with(".py") {
        let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir);
        return dir.rsplit('/').find(|segment| !segment.is_empty()).unwrap_or("default").to_string();
    }
    match file.find("/src/main/") {
        Some(at) => file[..at].rsplit('/').find(|s| !s.is_empty()).unwrap_or("default").to_string(),
        None => "default".to_string(),
    }
}

/// 参数：`file` 为文件路径。
/// 核心流程：去目录、去扩展名，得到控制器类名（Kotlin `Foo.kt` 与 Java 同名规则相同）。
pub fn class_name(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_string()
}

/// 参数：`module` 为模块名。
/// 核心流程：固定前缀 `m|`，避免与控制器/方法节点的 key 撞车。
pub fn module_key(module: &str) -> String {
    format!("m|{module}")
}

/// 参数：`file` 为文件路径。
/// 核心流程：固定前缀 `c|` 加完整路径。
pub fn class_key(file: &str) -> String {
    format!("c|{file}")
}

/// 参数：`route` 为一条路由。
/// 核心流程：`r|文件|行|方法|路径`，字段用 `|` 分隔，解析时限制字段数。
pub fn route_key(route: &Route) -> String {
    format!("r|{}|{}|{}|{}", route.file, route.line, route.method, route.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Route;

    fn route(file: &str, method: &str, path: &str, handler: &str, line: u32) -> Route {
        Route {
            method: method.into(),
            path: path.into(),
            handler: handler.into(),
            line,
            file: file.into(),
            unresolved: false,
            framework: "spring".into(),
        }
    }

    fn sample() -> RouteIndex {
        RouteIndex {
            routes: vec![
                route("/p/web/src/main/java/a/ShopController.java", "POST", "/shop/orders", "create", 12),
                route("/p/web/src/main/java/a/UserController.java", "GET", "/users/{id}", "get", 9),
                route("/p/core/src/main/java/b/CoreController.java", "GET", "/core/ping", "ping", 8),
                route("/p/web/src/main/java/a/UserController.java", "GET", "/users", "list", 5),
            ],
        }
    }

    #[test]
    fn sorts_modules_controllers_and_methods_stably() {
        let nodes = flatten(&sample());
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Found 4 services", "core", "CoreController", "[GET] ping", "web", "ShopController", "[POST] create", "UserController", "[GET] list", "[GET] get"]
        );
        // 同一份输入重排顺序后输出不变。
        let mut reversed = sample();
        reversed.routes.reverse();
        assert_eq!(flatten(&reversed), nodes);
    }

    #[test]
    fn keys_are_unique() {
        let nodes = flatten(&sample());
        let mut keys: Vec<&str> = nodes.iter().map(|n| n.key.as_str()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), nodes.len());
        assert_eq!(nodes.last().unwrap().key, "r|/p/web/src/main/java/a/UserController.java|9|GET|/users/{id}");
    }

    #[test]
    fn every_parent_exists_and_comes_first() {
        let nodes = flatten(&sample());
        for (i, node) in nodes.iter().enumerate() {
            let Some(parent) = &node.parent else {
                assert_eq!(node.key, ROOT);
                continue;
            };
            let at = nodes.iter().position(|n| &n.key == parent).expect("parent 必须在表里");
            assert!(at < i, "parent 必须更早：{}", node.key);
        }
    }

    #[test]
    fn grouping_by_controller_drops_the_module_level() {
        let nodes = group_by_controller(flatten(&sample()));
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Found 4 services",
                "CoreController",
                "[GET] ping",
                "ShopController",
                "[POST] create",
                "UserController",
                "[GET] list",
                "[GET] get",
            ]
        );
        assert!(nodes[1..].iter().all(|node| node.parent.as_deref() == Some(ROOT) || node.parent.as_deref().is_some_and(|p| p.starts_with("c|"))));
    }

    #[test]
    fn empty_index_has_only_the_root() {
        let nodes = flatten(&RouteIndex::default());
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].label, "Restful Routes: 0 routes", "空索引要给空态节点");
        assert_eq!(nodes[0].detail.as_deref(), Some(EMPTY_HINT), "空态要说明下一步怎么做");
        assert_eq!(nodes[0].parent, None, "空态节点仍是唯一的根");
        assert!(!nodes[0].expandable);
    }

    #[test]
    fn python_files_group_by_directory() {
        assert_eq!(module_of("app/routers/users.py"), "routers");
        assert_eq!(module_of("main.py"), "default");
        let index = RouteIndex {
            routes: vec![Route {
                method: "GET".into(),
                path: "/api/items".into(),
                handler: "items".into(),
                line: 3,
                file: "app/routers/items.py".into(),
                unresolved: false,
                framework: "fastapi".into(),
            }],
        };
        let labels: Vec<String> = flatten(&index).iter().map(|node| node.label.clone()).collect();
        assert_eq!(labels, ["Found 1 services", "routers", "items", "[GET] items"], "目录 → 文件名 → 函数");
    }

    #[test]
    fn module_comes_from_the_src_main_prefix() {
        assert_eq!(module_of("/p/billing/src/main/java/x/Y.java"), "billing");
        assert_eq!(module_of("src/main/java/x/Y.kt"), "default");
        assert_eq!(class_name("/p/a/UserController.kt"), "UserController");
    }
}
