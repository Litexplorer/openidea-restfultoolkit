//! 路由树视图（US-E2-01 / US-E2-02 / US-E2-03）：把 `tree` 的节点交给宿主渲染，
//! 展开事件只改本地展开集合后整棵重渲染，双击按 key 前缀决定展开还是跳源码。

use std::collections::BTreeSet;

use openidea_plugin::host;
use openidea_plugin::views::{Body, TreeItem, View, ViewEvent, Viewport, Views};
use openidea_plugin::{Error, Invocation};

use crate::RestfulToolkit;
use crate::tree::{self, Node};

/// 视图 id，必须与 plugin.toml 的 `[[contributes.views]]` 一致。
pub const VIEW_ID: &str = "restful-routes";

impl Views for RestfulToolkit {
    /// 参数：`view` 为视图 id（只有本插件这一个），`viewport` 不使用（树没有分页），`ctx` 为调用上下文。
    /// 核心流程：从 storage 读索引 → `tree::flatten` 摊平 → 截到 `MAX_NODES` →
    /// 按已展开集合标记 `expanded`；被截断时在状态行写明，不静默丢弃。
    fn render(&mut self, _view: &str, _viewport: Option<Viewport>, ctx: &Invocation) -> Result<View, Error> {
        // 索引为空时懒建一次（内部走 task，不在 5 s 预算里裸扫）。
        self.ensure_index(ctx);
        if self.is_building() && crate::load_index_for(crate::project_root(ctx).as_deref()).routes.is_empty() {
            return Ok(View::message(&crate::tr("Restful Routes: building the index…")));
        }
        let index = crate::load_index_for(crate::project_root(ctx).as_deref());
        let mut nodes = tree::flatten(&index);
        if host::setting("groupBy").is_some_and(|value| value.contains("controller")) {
            nodes = tree::group_by_controller(nodes);
        }
        let truncated = nodes.len() > tree::MAX_NODES;
        nodes.truncate(tree::MAX_NODES);
        let items: Vec<TreeItem> = nodes.iter().map(|node| item(node, &self.expanded)).collect();
        let mut view = View::new(Body::Tree(items));
        if truncated {
            view.status = Some(format!("已截断：只显示前 {} 个节点", tree::MAX_NODES));
        }
        Ok(view)
    }

    /// 参数：`view` 为视图 id，`event` 为宿主事件，`ctx` 为调用上下文。
    /// 核心流程：展开/收起只改本实例的展开集合并 `refresh_view` 重渲染；
    /// 激活按 key 前缀分流——模块展开、控制器与方法打开源码位置。
    fn handle(&mut self, _view: &str, event: ViewEvent, ctx: &Invocation) -> Result<(), Error> {
        match event {
            ViewEvent::Expand(key) => {
                if self.expanded.insert(key) {
                    host::refresh_view(VIEW_ID);
                }
            }
            ViewEvent::Collapse(key) => {
                if self.expanded.remove(&key) {
                    host::refresh_view(VIEW_ID);
                }
            }
            ViewEvent::Activate(key) => activate(ctx, &mut self.expanded, &key)?,
            _ => {}
        }
        Ok(())
    }
}

/// 参数：`ctx` 为调用上下文，`expanded` 为已展开节点 key 集合，`key` 为被激活节点的 key。
/// 核心流程：模块节点只展开并刷新；控制器节点开到文件首行；方法节点解析 `r|文件|行|…`
/// 后把 1 基行号换成 0 基再跳转。
fn activate(ctx: &Invocation, expanded: &mut BTreeSet<String>, key: &str) -> Result<(), Error> {
    let mut parts = key.splitn(5, '|');
    match parts.next() {
        Some("m") => {
            expanded.insert(key.to_string());
            host::refresh_view(VIEW_ID);
            Ok(())
        }
        Some("c") => {
            let file = parts.next().unwrap_or_default();
            let index = crate::load_index_for(crate::project_root(ctx).as_deref());
            let first = index.routes.iter().filter(|route| route.file == file).map(|route| route.line).min();
            match first {
                Some(line) => open(ctx, file, line.saturating_sub(1)),
                None => Err(Error::Failed("这个控制器下还没有路由，请先重建索引（Rebuild Route Index）".into())),
            }
        }
        Some("r") => {
            let file = parts.next().unwrap_or_default();
            let line = parts.next().and_then(|line| line.parse::<u32>().ok()).ok_or("节点 key 缺少行号，请刷新索引")?;
            if line == 0 {
                return Err(Error::Failed(crate::tr("Old index (no line numbers): rebuild the index")));
            }
            open(ctx, file, line.saturating_sub(1))
        }
        _ => Err(Error::Invalid(format!("未知的节点 key：{key}"))),
    }
}

/// 参数：`ctx` 为调用上下文，`file` 为源码路径，`line` 为 0 基行（列固定 0）。
/// 核心流程：先确认项目里还有这个文件（已改名或移出项目即视为失效）→
/// `host::open_location`；失效时换成可读提示，不打开错误位置。
fn open(ctx: &Invocation, file: &str, line: u32) -> Result<(), Error> {
    const STALE: &str = "源码位置已失效，请刷新索引（Restful Toolkit: Rebuild Route Index）";
    if let Some(project) = ctx.project() {
        match project.exists(file) {
            Ok(true) => {}
            Ok(false) | Err(Error::Invalid(_)) => return Err(Error::Failed(STALE.into())),
            Err(other) => return Err(other),
        }
    }
    host::open_location(file, line, 0).map_err(|error| match error {
        Error::NotFound(_) => Error::Failed(STALE.into()),
        other => other,
    })
}

/// 参数：`node` 为树节点，`expanded` 为已展开集合。
/// 核心流程：转成宿主 `TreeItem`；有父节点才设 `parent`，可展开节点带上展开状态。
fn item(node: &Node, expanded: &BTreeSet<String>) -> TreeItem {
    // 根节点文案在渲染期本地化（tree 是纯函数，不碰 host）。
    let label = if node.key == tree::ROOT {
        if node.label == "Restful Routes: 0 routes" {
            crate::tr("Restful Routes: 0 routes")
        } else {
            let count: String = node.label.chars().filter(|c| c.is_ascii_digit()).collect();
            crate::tr_fmt("Found {} services", &[&count])
        }
    } else {
        node.label.clone()
    };
    let mut item = TreeItem::new(&node.key, &label);
    if let Some(parent) = &node.parent {
        item = item.parent(parent);
    }
    if let Some(detail) = &node.detail {
        let detail = if node.key == tree::ROOT { crate::tr(detail) } else { detail.clone() };
        item = item.detail(&detail);
    }
    if node.expandable {
        item = item.expandable(expanded.contains(&node.key));
    }
    item
}

/// 参数：`ctx` 为调用上下文，`key` 为树节点 key（右键菜单的 Jump to Source 也走这里）。
/// 核心流程：与双击同一套分流逻辑，只是展开集合是临时的，不改变当前视图状态。
pub(crate) fn open_node(ctx: &Invocation, key: &str) -> Result<(), Error> {
    activate(ctx, &mut BTreeSet::new(), key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{Route, RouteIndex};
    use openidea_plugin::mock::MockHost;

    fn install(routes: Vec<Route>) -> MockHost {
        let host = MockHost::install();
        host.add_file("web/src/main/java/UserController.java", "class UserController {}\n");
        let index = RouteIndex { routes };
        let root = host.invocation().project().and_then(|project| project.root().ok()).unwrap_or_default();
        crate::save_index_for(Some(&root), &index).unwrap();
        host
    }


    fn route(path: &str, line: u32) -> Route {
        Route {
            method: "GET".into(),
            path: path.into(),
            handler: "get".into(),
            line,
            file: "/project/web/src/main/java/UserController.java".into(),
            unresolved: false,
            framework: "spring".into(),
        }
    }

    #[test]
    fn renders_the_three_levels_from_storage() {
        let host = install(vec![route("/api/users", 7)]);
        let view = RestfulToolkit::default().render(VIEW_ID, None, &host.invocation()).unwrap();
        let Body::Tree(items) = view.body else { panic!("应是树") };
        assert_eq!(items[0].label, "Found 1 services");
        assert_eq!(items[1].label, "web");
        assert_eq!(items[2].label, "UserController");
        assert_eq!(items[3].detail.as_deref(), Some("GET /api/users"));
        assert!(view.status.is_none());
    }

    #[test]
    fn activate_opens_the_zero_based_declaration_line() {
        let host = install(vec![route("/api/users", 7)]);
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(VIEW_ID, None, &host.invocation()).unwrap();
        let Body::Tree(items) = view.body else { panic!("应是树") };
        let key = items.last().unwrap().key.clone();
        plugin.handle(VIEW_ID, ViewEvent::Activate(key), &host.invocation()).unwrap();
        assert_eq!(host.opened_locations(), vec![("/project/web/src/main/java/UserController.java".into(), 6, 0)]);
    }

    #[test]
    fn module_activation_expands_instead_of_jumping() {
        let host = install(vec![route("/api/users", 7)]);
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(VIEW_ID, None, &host.invocation()).unwrap();
        let Body::Tree(items) = view.body else { panic!("应是树") };
        let module = items[1].key.clone();
        plugin.handle(VIEW_ID, ViewEvent::Activate(module.clone()), &host.invocation()).unwrap();
        assert!(host.opened_locations().is_empty());
        let view = plugin.render(VIEW_ID, None, &host.invocation()).unwrap();
        let Body::Tree(items) = view.body else { panic!("应是树") };
        assert!(items[1].expanded, "重渲染后模块仍是展开的");
    }

    #[test]
    fn controller_activation_jumps_to_its_first_declaration() {
        let host = install(vec![route("/api/users", 7)]);
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(VIEW_ID, None, &host.invocation()).unwrap();
        let Body::Tree(items) = view.body else { panic!("应是树") };
        let controller = items[2].key.clone();
        plugin.handle(VIEW_ID, ViewEvent::Activate(controller), &host.invocation()).unwrap();
        assert_eq!(
            host.opened_locations(),
            vec![("/project/web/src/main/java/UserController.java".into(), 6, 0)],
            "控制器跳第一条路由的声明行（1 基 7 → 0 基 6），不是文件首行"
        );
    }

    #[test]
    fn stale_source_location_is_a_readable_error() {
        let host = install(vec![route("/api/users", 7)]);
        let mut plugin = RestfulToolkit::default();
        let error = plugin
            .handle(VIEW_ID, ViewEvent::Activate("r|/project/gone/A.java|3|GET|/x".into()), &host.invocation())
            .unwrap_err();
        assert!(error.to_string().contains("已失效"), "{error}");
    }
}
