//! 路由详情（US-E2-02）：按选中的节点 key 产出 Method / Full URL / Params / Body 四个只读 tab。
//! 内容全部现算（URL 用当前 baseUrl/端口/context-path，参数与 body 来自源码）；缓存由调用方按 key 分桶。

use openidea_plugin::details::{Pane, Tab};
use openidea_plugin::Invocation;

use crate::{class_text, format, json_gen, load_index_for, params, project_root, tree, url};

/// 参数：`ctx` 为调用上下文，`key` 为树节点 key。
/// 核心流程：按 `tree::route_key` 找路由（只有方法节点有详情）→ 读源码取方法参数 →
/// 拼 Full URL、Params、Body；节点不是路由时给一句可读 Message。
pub(crate) fn tabs_for(ctx: &Invocation, key: &str) -> Vec<Tab> {
    let Some(route) = load_index_for(project_root(ctx).as_deref()).routes.into_iter().find(|route| tree::route_key(route) == key) else {
        return vec![Tab::new(
            "detail",
            "Detail",
            Pane::Message(crate::tr("This node has no route details: expand to a method node and select it")),
        )];
    };
    let project = ctx.project();
    let text = project.as_ref().and_then(|p| p.read_file(&route.file).ok()).unwrap_or_default();
    let base = project.as_ref().map_or_else(crate::default_base_url, |p| {
        crate::base_url(p).unwrap_or_else(|_| crate::default_base_url())
    });
    let parsed = params::parse(&crate::params_at(&text, route.line).unwrap_or_default());
    let lookup = |name: &str| class_text(project.as_ref(), name);
    let query = params::query_string(&parsed, lookup);
    let body = parsed
        .iter()
        .find(|param| param.body)
        .map_or_else(|| crate::tr("No request body (no @RequestBody parameter)"), |param| {
            json_gen::json_for_with(&param.ty, lookup, crate::json_options())
        });
    let params_text = if query.is_empty() { crate::tr("No query parameters") } else { query };
    vec![
        Tab::new("method", &crate::tr("Method"), Pane::Text(cap(route.method.clone()))),
        Tab::new("url", &crate::tr("Full URL"), Pane::Text(cap(url::route_url(&base, &route.framework, &route.path)))),
        Tab::new("params", &crate::tr("Params"), Pane::Text(cap(params_text))),
        Tab::new("body", &crate::tr("Body"), Pane::Text(cap(body))),
    ]
}

/// 参数：`text` 为要放进只读面板的文本。
/// 核心流程：按 `format::TEXT_LIMIT`（1 MiB）截断，超限时由 `truncate` 附「已截断」提示。
fn cap(text: String) -> String {
    format::truncate(&text, format::TEXT_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{Route, RouteIndex};
    use openidea_plugin::mock::MockHost;

    const ORDER: &str = "package demo;\n\n@RestController\n@RequestMapping(\"/api\")\npublic class OrderController {\n    @PostMapping(\"/v1/orders\")\n    public Order create(String name, int age, @RequestBody OrderCreateReq req) {\n        return null;\n    }\n}\n";
    const FILE: &str = "/project/src/OrderController.java";

    fn install() -> MockHost {
        let host = MockHost::install();
        host.add_file("src/OrderController.java", ORDER);
        host.add_file("src/OrderCreateReq.java", "public class OrderCreateReq {\n    private String sku;\n}\n");
        store(&host, vec![route("POST", "/api/v1/orders", "create", 6, FILE)]);
        host
    }

    fn route(method: &str, path: &str, handler: &str, line: u32, file: &str) -> Route {
        Route {
            method: method.into(),
            path: path.into(),
            handler: handler.into(),
            line,
            file: file.into(),
            unresolved: false,
            framework: "spring".into(),
            jump: None,
        }
    }

    fn current_index(host: &MockHost) -> crate::index::RouteIndex {
        let root = host.invocation().project().and_then(|project| project.root().ok()).unwrap_or_default();
        crate::load_index_for(Some(&root))
    }

    fn store(host: &MockHost, routes: Vec<Route>) {
        let root = host.invocation().project().and_then(|project| project.root().ok()).unwrap_or_default();
        crate::save_index_for(Some(&root), &RouteIndex { routes }).unwrap();
    }

    fn text(tab: &Tab) -> &str {
        let Pane::Text(text) = &tab.body else { panic!("{} 应是文本", tab.label) };
        text
    }

    #[test]
    fn a_method_node_has_the_four_tabs() {
        let host = install();
        let tabs = tabs_for(&host.invocation(), &tree::route_key(&current_index(&host).routes[0]));
        assert_eq!(tabs.iter().map(|tab| tab.label.as_str()).collect::<Vec<_>>(), ["Method", "Full URL", "Params", "Body"]);
        assert_eq!(text(&tabs[0]), "POST");
        assert_eq!(text(&tabs[1]), "http://localhost:8080/api/v1/orders");
    }

    #[test]
    fn params_and_body_come_from_the_source() {
        let host = install();
        let tabs = tabs_for(&host.invocation(), &tree::route_key(&current_index(&host).routes[0]));
        assert_eq!(text(&tabs[2]), "name=&age=0");
        assert_eq!(text(&tabs[3]), "{\n  \"sku\": \"demoData\"\n}");
    }

    #[test]
    fn every_tab_text_is_cut_at_the_one_mebibyte_limit() {
        let big = "x".repeat(format::TEXT_LIMIT + 10);
        let cut = cap(big);
        assert!(cut.len() < format::TEXT_LIMIT + 64, "截断后落在上限附近");
        assert!(cut.contains("已截断"), "要带提示");
        assert_eq!(cap("short".to_string()), "short", "短文本原样保留");
    }

    #[test]
    fn unknown_keys_get_a_readable_message() {
        let host = install();
        for key in ["root", "m|default", "c|/project/src/OrderController.java", "r|/gone.java|1|GET|/x"] {
            let tabs = tabs_for(&host.invocation(), key);
            assert_eq!(tabs.len(), 1, "{key}");
            assert!(matches!(tabs[0].body, Pane::Message(_)), "{key}");
        }
    }

    #[test]
    fn a_route_without_request_body_says_so() {
        let host = MockHost::install();
        let file = "/project/src/PlainController.java";
        host.add_file("src/PlainController.java", "public class PlainController {\n    @GetMapping(\"/ping\")\n    public String ping(int page) {\n        return null;\n    }\n}\n");
        store(&host, vec![route("GET", "/ping", "ping", 2, file)]);
        let tabs = tabs_for(&host.invocation(), &tree::route_key(&current_index(&host).routes[0]));
        assert!(text(&tabs[3]).contains("No request body"), "{}", text(&tabs[3]));
        assert_eq!(text(&tabs[2]), "page=0");
    }
}
