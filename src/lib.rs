//! Restful Toolkit：把 Java 源码里的 Spring 映射方法变成可直接粘贴的完整 URL，
//! 维护可持久化的路由索引，并把它变成可浏览、可跳转、可搜索的树视图
//! （US-E1-02 / US-E1-03 / US-E1-05 / US-E2-01..04 / US-E3-01）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use openidea_plugin::details::{Details, Pane, Tab};
use openidea_plugin::endpoints::{Endpoint as RouteEndpoint, EndpointsProvider};
use openidea_plugin::host;
use openidea_plugin::host::copy_to_clipboard;
use openidea_plugin::host::storage;
use openidea_plugin::search::{SearchItem, SearchPage, SearchProvider, SearchTarget};
use openidea_plugin::{
    Context, Error, Event, EventKind, HttpRequest, Invocation, Location, LogLevel, PickItem, Plugin, Position,
    Project, Range, StatusItem, TextEditor, register_plugin,
};

pub mod config;
pub mod detail;
pub mod fastapi;
pub mod format;
pub mod http;
pub mod index;
pub mod json_gen;
pub mod params;
pub mod scan;
pub mod tree;
pub mod url;
pub mod view;

/// 运行期文案的语言包（键是英文原文，值是该语言的文案）。
const LOCALE_EN: &str = include_str!("../locales/en.json");
const LOCALE_ZH: &str = include_str!("../locales/zh-CN.json");

/// 配置里读不到端口时的回落值。
const DEFAULT_PORT: u16 = 8080;

/// 配置与设置都没给出地址时的回落 base URL。
const DEFAULT_BASE_URL: &str = "http://localhost:8080";

/// 设置里没有 requestTimeoutMs 时的请求超时。
const DEFAULT_TIMEOUT_MS: u32 = 30_000;

/// 路由索引在 storage 里的键（视图、endpoints、搜索共用这一份缓存）。
pub(crate) const INDEX_KEY: &str = "restful.route-index";

/// 文件签名表（路径 → 内容哈希）的基键，增量刷新据此判断哪些文件需要重扫。
const SIGNATURE_KEY: &str = "restful.route-index.signatures";

/// 单次 find_files 的上限（大项目 2 千多文件时旧上限会截断）。
pub const FILE_CAP: u32 = 20_000;

/// 参数：`base` 为基键，`root` 为项目根（`None` 表示没有项目）。
/// 核心流程：把全局键变成「基键 + 项目根哈希前 16 位」的命名空间键，切项目不再串味。
fn scoped_key(base: &str, root: Option<&str>) -> String {
    format!("{base}.{:016x}", index::signature(root.unwrap_or("-")))
}

/// 参数：`ctx` 为调用上下文。
/// 核心流程：取当前项目根（没有项目或读不到时为 None），作为索引命名空间的依据。
pub(crate) fn project_root(ctx: &Invocation) -> Option<String> {
    ctx.project().and_then(|project| project.root().ok())
}

/// 命令 id：复制完整 URL。
const CMD_COPY_FULL_URL: &str = "copy-full-url";

/// 命令 id：重建路由索引。
const CMD_REFRESH_INDEX: &str = "refresh-index";

/// 命令 id：按 URL 片段搜索路由（US-E2-04 的命令入口）。
const CMD_SEARCH_ROUTES: &str = "search-routes";

/// 命令 id：复制 query 参数骨架（US-E3-02）。
const CMD_COPY_QUERY_PARAM: &str = "copy-query-param";

/// 命令 id：复制请求体 JSON 骨架（US-E3-03）。
const CMD_COPY_REQUEST_BODY: &str = "copy-request-body";

/// 命令 id：复制树节点的完整 URL（US-E3-04 的右键菜单）。
const CMD_COPY_URL: &str = "copy-url";

/// 命令 id：从树节点跳到源码（US-E3-04 的右键菜单）。
const CMD_OPEN_SOURCE: &str = "open-source";

/// 命令 id：多选树节点后批量复制 URL（US-E3-04）。
const CMD_COPY_MANY_URLS: &str = "copy-many-urls";

/// 命令 id：聚合入口（拷 URL / 拷 Query / 拷 Body），默认快捷键 Alt+Shift+R。
const CMD_PICK: &str = "restful-pick";

/// 命令 id：设置页的「测试解析」按钮。
const CMD_TEST_PARSE: &str = "test-parse";

/// 命令 id：快捷键弹出候选条，筛选后跳到路由声明（US-E2-04 的旧插件手感）。
const CMD_GOTO_ROUTE: &str = "goto-route";

/// 候选条一次最多喂多少条（超出只喂前 N 条并在标题注明截断）。
pub const GOTO_LIMIT: usize = 2000;

/// 命令 id：发送一次请求并把响应放进 Response 面板（US-E4-01）。
const CMD_SEND_REQUEST: &str = "send-request";

/// 命令 id：光标所在类 → 示例 JSON（US-E5-01）。
const CMD_CONVERT_JSON: &str = "convert-json";

/// 命令 id：光标所在类 → 压缩 JSON（US-E5-02）。
const CMD_CONVERT_JSON_COMPRESSED: &str = "convert-json-compressed";

/// 命令 id：光标所在类 → Bulk Value（US-E5-03）。
const CMD_CONVERT_BULK_VALUE: &str = "convert-bulk-value";

#[derive(Default)]
pub struct RestfulToolkit {
    /// 树视图已展开的节点 key：宿主不记忆展开状态，重渲染时靠它复原。
    expanded: BTreeSet<String>,
    /// 最近一次请求的展示内容（详情面板末尾追加的 Response tab）。
    response: Option<ResponseView>,
    /// 详情面板缓存，按「视图 id|节点 key」分桶：多项目/多窗口各一份实例，互不覆盖。
    details: BTreeMap<String, Vec<Tab>>,
    /// storage 被拒时的会话内索引：不落盘，但本次运行期间 endpoints/详情仍可用。
    session_index: Option<(String, index::RouteIndex)>,
    /// 单飞标志：已有全量/增量任务在跑时为 true，新触发直接返回（不阻塞 UI）。
    building: AtomicBool,
    /// 去抖缓冲：500 ms 内到达的变更攒在这里，下一批一起扫。
    pending: Vec<(Vec<String>, Vec<String>, Vec<String>)>,
    /// 上次真正扫盘的时间戳（毫秒），用于去抖。
    last_flush: u64,
    /// `on-startup-finished` 登记的「下次有 Invocation 时做一次签名校验」。
    verify_pending: bool,
}

/// 最近一次请求的展示内容；文本都按 `format::TEXT_LIMIT` 截断过。
#[derive(Clone, Debug, Default)]
pub(crate) struct ResponseView {
    pub text: String,
    pub headers: String,
}

impl Plugin for RestfulToolkit {
    /// 参数：`_ctx` 为激活上下文。
    /// 核心流程：订阅 `files-changed` 让索引随文件改动增量更新；没有 `events` 权限时安静跳过，
    /// 仍可用 `refresh-index` 手动重建。
    fn activate(&mut self, _ctx: &Context) -> Result<(), Error> {
        let subscribed = match host::subscribe(EventKind::FilesChanged) {
            Ok(()) | Err(Error::Denied(_)) => Ok(()),
            Err(other) => Err(other),
        };
        // on-startup-finished 只做签名校验：索引非空时只重扫哈希变了的文件，绝不整份重扫。
        // 启动后不做全量扫：只登记一次签名校验，等第一次拿到 Invocation（视图/命令）再跑。
        self.verify_pending = true;
        subscribed
    }

    /// 参数：`events` 为订阅到的事件批次，`ctx` 为调用上下文。
    /// 核心流程：`files-changed` 事件里新增/修改的 `.java`/`.kt` 只重扫该文件，删除的移除条目，
    /// 再把索引写回 storage 并刷新视图；读写失败不 panic。
    fn handle_events(&mut self, events: &[Event], ctx: &Invocation) {
        let mut queued = false;
        for event in events {
            if let Event::FilesChanged(changes) = event {
                self.pending
                    .push((changes.created.clone(), changes.modified.clone(), changes.removed.clone()));
                queued = true;
            }
        }
        if !queued {
            return;
        }
        // 去抖：500 ms 内的变更先攒着，合并成一批再扫（不 sleep、不阻塞）。
        if !self.should_flush() {
            return;
        }
        self.flush_pending(ctx);
    }

    /// 参数：`id` 为命令 id，`ctx` 为调用上下文，`arg` 本迭代不用（相对 URL 留给后续迭代）。
    /// 核心流程：按 id 分发到复制或重建索引 → 成功时回报一句文案；未知命令、缺编辑器、
    /// 没解析到映射、缺项目都返回可读错误，不 panic。
    fn run_command(&mut self, id: &str, ctx: &Invocation, arg: Option<&str>) -> Result<Option<String>, Error> {
        match id {
            CMD_COPY_FULL_URL => {
                let (endpoint, full_url) = resolve(ctx)?;
                let (copied, note) = if endpoint.unresolved {
                    (
                        format!("{full_url}\n\n⚠ {}：{}", tr("Path not statically resolved"), endpoint.path),
                        "（unresolved）",
                    )
                } else {
                    (full_url.clone(), "")
                };
                let missing = http::missing_vars(&full_url, &[]);
                let copied = if missing.is_empty() {
                    copied
                } else {
                    let hint = tr_fmt("Missing path variables: {}", &[&missing.join(", ")]);
                    format!("{copied}\n\n⚠ {hint}")
                };
                let copied_note = copy_text(&copied)?;
                let message = format!(
                    "{copied_note} {:?} {}{note}（{}，第 {} 行）",
                    endpoint.http_method, full_url, endpoint.handler, endpoint.line
                );
                Ok(Some(message))
            }
            CMD_REFRESH_INDEX => {
                self.details.clear();
                let (index, message, persisted) = refresh_index(ctx, "command")?;
                let root = project_root(ctx).unwrap_or_default();
                self.session_index = (!persisted).then_some((root, index));
                Ok(Some(message))
            }
            CMD_PICK => Ok(Some(pick_command(ctx)?)),
            CMD_TEST_PARSE => Ok(Some(test_parse(ctx)?)),
            CMD_GOTO_ROUTE => Ok(Some(self.goto_route(ctx)?)),
            CMD_COPY_QUERY_PARAM => Ok(Some(copy_query(ctx)?)),
            CMD_COPY_REQUEST_BODY => Ok(Some(copy_body(ctx)?)),
            CMD_CONVERT_JSON => Ok(Some(convert(ctx, Convert::Json)?)),
            CMD_CONVERT_JSON_COMPRESSED => Ok(Some(convert(ctx, Convert::Compressed)?)),
            CMD_CONVERT_BULK_VALUE => Ok(Some(convert(ctx, Convert::Bulk)?)),
            CMD_COPY_URL => Ok(Some(copy_node_urls(ctx, arg.ok_or("请从路由树节点右键运行此命令")?)?)),
            CMD_COPY_MANY_URLS => Ok(Some(copy_many_urls(ctx)?)),
            CMD_SEND_REQUEST => Ok(Some(self.send(ctx, arg)?)),
            CMD_OPEN_SOURCE => {
                view::open_node(ctx, arg.ok_or("请从路由树节点右键运行此命令")?)?;
                Ok(None)
            }
            CMD_SEARCH_ROUTES => {
                let query = arg.unwrap_or_default();
                let root = project_root(ctx);
                let lines = search(&load_index_for(root.as_deref()), query, 200);
                if lines.is_empty() {
                    return Err(Error::Failed(format!("没有匹配「{query}」的路由，请先重建索引")));
                }
                Ok(Some(lines.join("\n")))
            }
            _ => Err(Error::NotFound(format!("command {id}"))),
        }
    }
}

register_plugin!(RestfulToolkit, views, endpoints, details, search);

impl SearchProvider for RestfulToolkit {
    /// 参数：`_category` 为分类 id（本插件只有 routes），`query` 为 URL 片段（可带 `*`），
    /// `limit` 为条数上限，`_ctx` 未用（只读内存/缓存索引）。
    /// 核心流程：完整 URL 先剥成路径 → 通配或子串匹配（大小写不敏感）→ 按路径排序 →
    /// 生成可直接跳到声明行的 `Location` 结果，首条超限时置 `has_more`。
    fn search(&mut self, _category: &str, query: &str, limit: u32, ctx: &Invocation) -> Result<SearchPage, Error> {
        let mut needle = path_part(query);
        // 贴进来的完整 URL 常带本项目的 context-path，先剥掉再按路由路径匹配。
        let context = context_path(ctx);
        if !context.is_empty() {
            if let Some(rest) = needle.strip_prefix(&context) {
                needle = rest.to_string();
            }
        }
        let limit = limit.clamp(1, 200) as usize;
        let mut hits: Vec<index::Route> =
            load_index_for(project_root(ctx).as_deref()).routes.into_iter().filter(|route| matches_query(&route.path, &needle)).collect();
        if needle.is_empty() {
            // 空查询只给前若干条建议：按模块名分组，模块内再按路径与方法。
            hits.sort_by(|a, b| {
                tree::module_of(&a.file)
                    .cmp(&tree::module_of(&b.file))
                    .then_with(|| a.path.cmp(&b.path))
                    .then_with(|| a.method.cmp(&b.method))
            });
        } else {
            hits.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)));
        }
        let has_more = hits.len() > limit;
        let items = hits
            .into_iter()
            .take(limit)
            .map(|route| {
                let line = route.line.saturating_sub(1);
                SearchItem {
                    id: tree::route_key(&route),
                    title: format!("{} {}", route.method, route.path),
                    description: Some(format!("{}:{}", route.file, route.line)),
                    target: SearchTarget::Location(Location {
                        path: route.file.clone(),
                        range: Range::new(Position::new(line, 0), Position::new(line, 0)),
                    }),
                }
            })
            .collect();
        Ok(SearchPage { items, has_more })
    }
}

/// 参数：`ctx` 为调用上下文。
/// 核心流程：由当前项目的 base URL 取出 context-path（`http://h:8080/gateway` → `/gateway`）；
/// 没有项目、没有 context-path 或取不到配置时返回空串。
fn context_path(ctx: &Invocation) -> String {
    let Some(project) = ctx.project() else { return String::new() };
    let Ok(base) = base_url(&project) else { return String::new() };
    match base.split_once("://").map(|(_, rest)| rest).and_then(|rest| rest.find('/').map(|at| rest[at..].to_string())) {
        Some(path) if path != "/" => path,
        _ => String::new(),
    }
}

/// 参数：`query` 为搜索词，可能是一整条 URL。
/// 核心流程：有 `://` 时去掉 scheme/host/port 只留路径；否则原样返回。
fn path_part(query: &str) -> String {
    match query.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or_else(String::new, |at| rest[at..].to_string()),
        None => query.to_string(),
    }
}

/// 参数：`path` 为路由路径，`needle` 为搜索词（为空表示要默认建议）。
/// 核心流程：空查询全命中；含 `*` 走通配（`*` 匹配任意串），否则大小写不敏感子串。
fn matches_query(path: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let path = path.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.contains('*') {
        wildcard(&needle, &path)
        } else {
        path.contains(&needle) || needle.contains(&path) || needle.contains(literal_prefix(&path))
    }
}

/// 参数：`path` 为路由路径。
/// 核心流程：截到第一个 `{` 之前（去掉尾部 `/`），用来把「贴进来的完整 URL」按字面前缀命中。
fn literal_prefix(path: &str) -> &str {
    match path.find('{') {
        Some(at) => path[..at].trim_end_matches('/'),
        None => path,
    }
}

/// 参数：`pattern` 为含 `*` 的模式，`text` 为被匹配的小写文本。
/// 核心流程：贪心 + 回退（记住最后一个 `*` 的位置），线性时间内判定，不递归不爆栈。
fn wildcard(pattern: &str, text: &str) -> bool {
    let (pattern, text): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

impl Details for RestfulToolkit {
    /// 参数：`view` 为视图 id，`key` 为选中节点的 key，`ctx` 为调用上下文。
    /// 核心流程：按「视图|key」查本实例的缓存，未命中再调 `detail::tabs_for` 并写回；
    /// 最近有响应时在末尾追加 Response/Headers tab（≤1 MiB），不覆盖路由详情。
    fn detail(&mut self, view: &str, key: &str, ctx: &Invocation) -> Result<Vec<Tab>, Error> {
        let cache_key = format!("{view}|{key}");
        let mut tabs = match self.details.get(&cache_key) {
            Some(tabs) => tabs.clone(),
            None => {
                let tabs = detail::tabs_for(ctx, key);
                self.details.insert(cache_key, tabs.clone());
                tabs
            }
        };
        if let Some(response) = &self.response {
            tabs.push(Tab::new("response", "Response", Pane::Text(response.text.clone())));
            tabs.push(Tab::new("headers", "Headers", Pane::Text(format::truncate(&response.headers, format::TEXT_LIMIT))));
        }
        Ok(tabs)
    }
}

impl RestfulToolkit {
    /// 参数：`ctx` 为调用上下文。
    /// 核心流程：取索引（空则先懒建一次；构建中给可读错误）→ 一步弹候选条 → 选中后跳到声明行。
    fn goto_route(&mut self, ctx: &Invocation) -> Result<String, Error> {
        self.ensure_index(ctx);
        let index = load_index_for(project_root(ctx).as_deref());
        if index.routes.is_empty() {
            if self.is_building() {
                return Err(Error::Failed(tr("Restful Routes: building the index…")));
            }
            return Err(Error::Failed(tr("Index is empty: run Rebuild Route Index first")));
        }
        let (routes, truncated, total) = goto_candidates(&index);
        let title = if truncated {
            tr_fmt("Restful Routes ({} of {} shown)", &[&GOTO_LIMIT.to_string(), &total.to_string()])
        } else {
            tr("Restful Routes")
        };
        let files: Vec<&str> = index.routes.iter().map(|route| route.file.as_str()).collect();
        let subs = tree::subprojects(&files);
        let items: Vec<PickItem> = routes.iter().map(|route| goto_item(route, &subs)).collect();
        let Some(key) = host::pick(&title, &items)? else { return Err(Error::Cancelled) };
        let Some(route) = index.routes.iter().find(|route| tree::route_key(route) == key) else {
            return Err(Error::Failed(tr("No routes match the filter")));
        };
        if route.line == 0 {
            return Err(Error::Failed(tr("Old index (no line numbers): rebuild the index")));
        }
        let line = route.line.saturating_sub(1);
        host::open_location(&route.file, line, 0).map_err(denied_hint)?;
        Ok(format!("{}:{line}", route.file))
    }

    /// 参数：`ctx` 为调用上下文。
    /// 核心流程：视图/命令在索引为空时懒建一次（走 `refresh_index`，带 task 与进度）；
    /// 已有索引或已有任务在跑时直接跳过，保证大项目不会在 5 s 预算里裸扫。
    pub(crate) fn ensure_index(&mut self, ctx: &Invocation) {
        let (_diag, _started) = begin_diagnostics("view", ctx);
        if self.verify_pending {
            self.verify_pending = false;
            self.verify_signatures(ctx);
        }
        let root = project_root(ctx);
        let pending = storage::get(&scoped_key(PENDING_KEY, root.as_deref())).ok().flatten().is_some();
        if (!load_index_for(root.as_deref()).routes.is_empty() && !pending) || self.is_building() {
            return;
        }
        if !self.begin_build() {
            return;
        }
        let _ = refresh_index(ctx, "view");
        self.end_build();
    }

    /// 参数：`ctx` 为调用上下文。
    /// 核心流程：只在索引非空时做签名校验——`refresh_index` 本身按签名 diff，只重扫哈希变化的
    /// 文件；索引为空时不做事（首次建立交给 `ensure_index` 的懒触发）。
    fn verify_signatures(&mut self, ctx: &Invocation) {
        if load_index_for(project_root(ctx).as_deref()).routes.is_empty() || !self.begin_build() {
            return;
        }
        let _ = refresh_index(ctx, "startup");
        self.end_build();
    }

    /// 参数：无。
    /// 核心流程：原子地抢占单飞标志；已有任务在跑时返回 false。
    fn begin_build(&self) -> bool {
        !self.building.swap(true, Ordering::SeqCst)
    }

    /// 参数：无。
    /// 核心流程：释放单飞标志。
    fn end_build(&self) {
        self.building.store(false, Ordering::SeqCst);
    }

    /// 参数：无。
    /// 核心流程：是否有全量/增量扫描在跑（视图与命令据此显示「正在建立索引…」）。
    pub(crate) fn is_building(&self) -> bool {
        self.building.load(Ordering::SeqCst)
    }

    /// 参数：无。
    /// 核心流程：距上次扫盘是否已过 `DEBOUNCE_MS`；过了就更新时间戳并返回 true。
    fn should_flush(&mut self) -> bool {
        let now = now_ms();
        if now.saturating_sub(self.last_flush) < DEBOUNCE_MS {
            return false;
        }
        self.last_flush = now;
        true
    }

    /// 参数：`ctx` 为调用上下文。
    /// 核心流程：单飞保护下把攒下的变更批量应用；拿不到单飞就留着下一批再试。
    fn flush_pending(&mut self, ctx: &Invocation) {
        if self.pending.is_empty() || !self.begin_build() {
            return;
        }
        for (created, modified, removed) in std::mem::take(&mut self.pending) {
            self.apply_file_changes(&created, &modified, &removed, ctx);
        }
        self.end_build();
    }

    /// 参数：`created`/`modified`/`removed` 为宿主给出的路径，`ctx` 为调用上下文。
    /// 核心流程：只处理 `.java`/`.kt`；读得到就 `update_file`（只重扫这一个文件），
    /// 删掉的 `remove_file`；随后持久化并刷新视图。
    fn apply_file_changes(&mut self, created: &[String], modified: &[String], removed: &[String], ctx: &Invocation) {
        let (mut diag, started) = begin_diagnostics("event", ctx);
        if created.is_empty() && modified.is_empty() && removed.is_empty() {
            diag.error = Some("files-changed batch is empty".to_string());
            finish_diagnostics(diag, started);
            return;
        }
        let root = project_root(ctx);
        let graph = load_prefix_graph(root.as_deref());
        let mut index = load_index_for(root.as_deref());
        let mut signatures = load_signatures_for(root.as_deref());
        let mut touched = false;
        if let Some(project) = ctx.project() {
            for path in created.iter().chain(modified).filter(|path| is_source(path)) {
                if is_excluded_file(path) || !matches_project_prefix(path) {
                    index.remove_file(path);
                    continue;
                }
                if let Ok(text) = project.read_file(path) {
                    signatures.insert(path.clone(), index::signature(&text));
                    let endpoints = if path.ends_with(".py") {
                        scan::scan_fastapi_with(&text, Some(&|receiver: &str| graph.prefix_of(path, receiver)))
                    } else {
                        scan::scan_any(path, &text)
                    };
                    index.update_file_scanned(path, &text, endpoints);
                    touched = true;
                }
            }
            for path in removed.iter().filter(|path| is_source(path)) {
                signatures.remove(path);
                index.remove_file(path);
                touched = true;
            }
        }
        if !touched {
            diag.error = Some(if root.is_none() {
                "未拿到项目根（window 未就绪或项目正在索引），请稍后重试".to_string()
            } else {
                "files-changed 里没有可读的 .java/.kt".to_string()
            });
            finish_diagnostics(diag, started);
            return;
        }
        diag.files_to_parse = created.iter().chain(modified.iter()).filter(|path| is_source(path)).count();
        diag.storage_written = save_signatures_for(root.as_deref(), &signatures).unwrap_or(false)
            && save_index_for(root.as_deref(), &index).is_ok();
        diag.routes = index.routes.len();
        finish_diagnostics(diag, started);
        host::refresh_view(view::VIEW_ID);
    }
}

/// 参数：`path` 为文件路径。
/// 核心流程：`.java` 或 `.kt` 才算路由来源。
fn is_source(path: &str) -> bool {
    path.ends_with(".java") || path.ends_with(".kt") || path.ends_with(".py")
}

/// endpoints 一次最多交给宿主的条数（超过会被整份作废）。
pub const ENDPOINT_LIMIT: usize = 10_000;

/// 参数：`index` 为路由索引。
/// 核心流程：映射成宿主的 `Endpoint`（0 基行、framework 原样带上）→ 超过 `ENDPOINT_LIMIT`
/// 截断并返回「是否被截断」，调用方据此记日志提示，不静默丢弃。
pub(crate) fn endpoints_of(index: &index::RouteIndex) -> (Vec<RouteEndpoint>, bool) {
    let truncated = index.routes.len() > ENDPOINT_LIMIT;
    let endpoints = index
        .routes
        .iter()
        .take(ENDPOINT_LIMIT)
        .map(|route| {
            let line = route.line.saturating_sub(1);
            RouteEndpoint {
                method: (route.method != "ANY").then(|| route.method.clone()),
                path: route.path.clone(),
                framework: route.framework.clone(),
                handler: Some(route.handler.clone()),
                location: Location {
                    path: route.file.clone(),
                    range: Range::new(Position::new(line, 0), Position::new(line, 0)),
                },
            }
        })
        .collect();
    (endpoints, truncated)
}

impl EndpointsProvider for RestfulToolkit {
    /// 参数：`ctx` 为宿主给的显式项目调用（索引按项目根命名空间存放）。
    /// 核心流程：只读当前项目的缓存（或会话内索引）→ `endpoints_of` 映射并截到 1 万条；
    /// 被截断时记一条日志提示，缓存缺失或损坏时返回空表，不报错也不 panic。
    fn endpoints(&mut self, ctx: &Invocation) -> Result<Vec<RouteEndpoint>, Error> {
        let root = project_root(ctx);
        let index = match &self.session_index {
            Some((cached_root, index)) if Some(cached_root.as_str()) == root.as_deref() => index.clone(),
            _ => load_index_for(root.as_deref()),
        };
        let (endpoints, truncated) = endpoints_of(&index);
        if endpoints.is_empty() {
            // endpoints 只能用空表表达「没有路由」，再挂一条状态栏文案，让人知道插件是活的。
            let item = StatusItem {
                text: tr("Restful Routes: 0 routes"),
                tooltip: Some(tr(crate::tree::EMPTY_HINT)),
                command: Some(CMD_REFRESH_INDEX.to_string()),
            };
            let _ = host::set_window_status_item("restful-routes", &item);
        }
        if truncated {
            host::log(
                LogLevel::Warn,
                &format!("routes exceed {ENDPOINT_LIMIT}: only the first {ENDPOINT_LIMIT} are published"),
            );
        }
        Ok(endpoints)
    }
}

/// 参数：`root` 为项目根（`None` 表示没有项目）。
/// 核心流程：取该项目的索引键 → `RouteIndex::from_json`；缺失或损坏一律给空索引。
pub(crate) fn load_index_for(root: Option<&str>) -> index::RouteIndex {
    let Some(json) = storage::get(&scoped_key(INDEX_KEY, root)).ok().flatten() else {
        return index::RouteIndex::default();
    };
    match serde_json::from_str::<StoredIndex>(&json) {
        Ok(stored) if stored.parser_version == PARSER_VERSION => index::RouteIndex { routes: stored.routes },
        _ => index::RouteIndex::default(),
    }
}

/// 参数：`root` 为项目根。
/// 核心流程：读落盘索引的解析器版本；缺失/损坏返回空串（视同旧版本）。
fn stored_parser_version(root: Option<&str>) -> String {
    storage::get(&scoped_key(INDEX_KEY, root))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<StoredIndex>(&json).ok())
        .map(|stored| stored.parser_version)
        .unwrap_or_default()
}

/// 参数：`root` 为项目根。
/// 核心流程：版本不符时清空签名表与前缀图，让下一次重建必然全量重解析。
fn invalidate_derived(root: Option<&str>) -> bool {
    if stored_parser_version(root) == PARSER_VERSION {
        return false;
    }
    let _ = storage::delete(&scoped_key(SIGNATURE_KEY, root));
    let _ = storage::delete(&scoped_key(PREFIX_GRAPH_KEY, root));
    let _ = storage::delete(&scoped_key(INDEX_KEY, root));
    true
}

/// 参数：`root` 为项目根，`graph` 为前缀图。
/// 核心流程：序列化到该项目的命名空间键；没有 storage 权限时静默跳过（下次重建会重算）。
fn save_prefix_graph(root: Option<&str>, graph: &fastapi::PrefixGraph) -> Result<(), Error> {
    let stored = StoredGraph { parser_version: PARSER_VERSION.to_string(), graph: graph.clone() };
    let json = serde_json::to_string(&stored).map_err(|error| Error::Failed(format!("前缀图序列化失败：{error}")))?;
    match storage::set(&scoped_key(PREFIX_GRAPH_KEY, root), &json) {
        Ok(()) | Err(Error::Denied(_)) => Ok(()),
        Err(other) => Err(other),
    }
}

/// 参数：`root` 为项目根。
/// 核心流程：读该项目的图；缺失或损坏给空图（此时 Python 路由退回本文件前缀并标 unresolved）。
fn load_prefix_graph(root: Option<&str>) -> fastapi::PrefixGraph {
    storage::get(&scoped_key(PREFIX_GRAPH_KEY, root))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<StoredGraph>(&json).ok())
        .filter(|stored| stored.parser_version == PARSER_VERSION)
        .map(|stored| stored.graph)
        .unwrap_or_default()
}

/// 参数：`root` 为项目根，`index` 为要落盘的索引。
/// 核心流程：写到该项目的命名空间键（`Denied` 由调用方决定是否降级为会话内索引）。
pub(crate) fn save_index_for(root: Option<&str>, index: &index::RouteIndex) -> Result<(), Error> {
    let stored = StoredIndex { parser_version: PARSER_VERSION.to_string(), routes: index.routes.clone() };
    let json = serde_json::to_string(&stored).map_err(|error| Error::Failed(format!("索引序列化失败：{error}")))?;
    storage::set(&scoped_key(INDEX_KEY, root), &json)
}

/// 参数：`index` 为路由索引，`query` 为 URL 片段（空串给默认建议），`limit` 为条数上限。
/// 核心流程：大小写不敏感地匹配路径/方法/处理方法名 → 按路径与方法排序 →
/// 取前 `limit` 条，每条格式化成「方法 路径 ← 文件:行」，供命令直接展示。
pub(crate) fn search(index: &index::RouteIndex, query: &str, limit: usize) -> Vec<String> {
    let needle = query.trim().to_lowercase();
    let mut hits: Vec<&index::Route> = index
        .routes
        .iter()
        .filter(|route| {
            needle.is_empty()
                || route.path.to_lowercase().contains(&needle)
                || route.method.to_lowercase().contains(&needle)
                || route.handler.to_lowercase().contains(&needle)
        })
        .collect();
    hits.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)));
    hits.into_iter()
        .take(limit)
        .map(|route| format!("{} {}  ← {}:{}", route.method, route.path, route.file, route.line))
        .collect()
}

impl RestfulToolkit {
    /// 参数：`ctx` 为命令调用上下文，`input` 为 `METHOD URL` 或只有 `URL`（留空则用当前端点的 URL）。
    /// 核心流程：开 task 把预算续到 60 s → `host::fetch` → 状态、Content-Type 与美化后的 body
    /// 截到 1 MiB 放进详情面板 → 返回一句状态文案；权限/网络错误换成可读提示，不 panic。
    fn send(&mut self, ctx: &Invocation, input: Option<&str>) -> Result<String, Error> {
        let (method, url, body) = match input.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => {
                let (method, url, pairs) = http::parse_request_input(text);
                let mut query = Vec::new();
                for (key, value) in &pairs {
                    if !url.contains(&format!("{{{key}}}")) {
                        query.push(format!("{key}={value}"));
                    }
                }
                let url = http::apply_path_vars(&url, &pairs);
                (method, http::append_query(&url, &query.join("&")), None)
            }
            None => request_from_endpoint(ctx)?,
        };
        let task = host::begin_task("Restful Toolkit: 发送请求", true);
        task.report(0, None, Some("正在发送…"));
        let timeout = setting_u32("requestTimeoutMs", DEFAULT_TIMEOUT_MS);
        let request = HttpRequest {
            method: method.clone(),
            url: url.clone(),
            headers: extra_headers(),
            body: body.map(String::into_bytes),
            timeout_ms: Some(timeout),
        };
        let response = host::fetch(&request).map_err(|error| explain_fetch(error, &url))?;
        task.report(1, Some(1), Some("已收到响应"));
        let content_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map_or("未知", |(_, value)| value.as_str());
        let body = response.text();
        let body = format::pretty_json(&body).unwrap_or(body);
        let text = format!(
            "{method} {url}\n状态：{}\nContent-Type：{content_type}\n\n{body}",
            response.status
        );
        self.response = Some(ResponseView {
            text: format::truncate(&text, format::TEXT_LIMIT),
            headers: response.headers.iter().map(|(name, value)| format!("{name}: {value}")).collect::<Vec<_>>().join("\n"),
        });
        host::refresh_view(view::VIEW_ID);
        Ok(format!("{method} {url} → 状态 {}，响应见 Response 面板", response.status))
    }
}

/// 参数：`ctx` 为调用上下文。
/// 核心流程：取当前端点的完整 URL → `apply_path_vars`（当前没有变量表，占位符原样保留）→
/// `append_query` 拼上方法参数的 query 骨架 → body 取 `@RequestBody` 的示例 JSON。
fn request_from_endpoint(ctx: &Invocation) -> Result<(String, String, Option<String>), Error> {
    let (text, endpoint) = current_method(ctx)?;
    let project = ctx.project();
    let lookup = |name: &str| class_text(project.as_ref(), name);
    let list = params::parse(&params_at(&text, endpoint.line).unwrap_or_default());
    let base = match &project {
        Some(project) => base_url(project)?,
        None => default_base_url(),
    };
    let base_path = index::class_path_above(&text, endpoint.line);
    let url = url::join_url(&base, &base_path, &endpoint.path);
    let url = http::apply_path_vars(&url, &[]);
    let url = http::append_query(&url, &params::query_string(&list, lookup));
    let body = list.iter().find(|param| param.body).map(|param| json_gen::json_for(&param.ty, lookup));
    Ok((index::verb(&endpoint.http_method).to_string(), url, body))
}

/// 参数：`error` 为 `net.fetch` 的失败原因，`url` 为请求地址。
/// 核心流程：`Denied` 补上缺的权限、开在哪、怎么改；其余错误原样返回由宿主展示。
fn explain_fetch(error: Error, url: &str) -> Error {
    let host = url.split("://").nth(1).and_then(|rest| rest.split(['/', ':']).next()).unwrap_or("localhost");
    match error {
        Error::Denied(cap) => Error::Failed(format!(
            "{}：{cap}（{} net.http:{host}）",
            tr("Request failed"),
            tr("Open Settings > Plugins > Restful Toolkit and allow")
        )),
        other => other,
    }
}

/// 类 → 文本的三种输出形态（US-E5-01/02/03）。
#[derive(Clone, Copy)]
enum Convert {
    Json,
    Compressed,
    Bulk,
}

/// 参数：`ctx` 为命令调用上下文，`mode` 为输出形态。
/// 核心流程：取编辑器全文 → 只留类体顶层的行 → 按形态调用 `json_gen` → 写剪贴板；
/// 光标不在类里时返回可读错误，不写剪贴板。
fn convert(ctx: &Invocation, mode: Convert) -> Result<String, Error> {
    let editor = ctx.editor().ok_or("请在 Java/Kotlin 编辑器中运行：当前没有可读取的文件")?;
    let text = editor.document()?.text;
    let body = class_body_at(&text, cursor_line(&editor)).ok_or("把光标放在类名或类体内：没找到 class/record/enum 声明")?;
    let project = ctx.project();
    let lookup = |name: &str| class_text(project.as_ref(), name);
    let options = json_options();
    let output = match mode {
        Convert::Json => json_gen::class_json_with(&body, lookup, options),
        Convert::Compressed => json_gen::to_compressed(&json_gen::class_json_with(&body, lookup, options)),
        Convert::Bulk => {
            let chain = parent_chain(&text, &lookup);
            json_gen::to_bulk_value_full(&body, &chain, &line_ending(), options)
        }
    };
    let note = copy_text(&output)?;
    Ok(format!("{note} {} B", output.len()))
}

/// 参数：`text` 为源码全文，`line` 为光标行（1 基）。
/// 核心流程：用花括号配对找出所有类型声明的行区间 → 取包含光标的最内层类体；
/// 没有包含光标的就回落第一个类体；一个类型声明都没有返回 None。
fn class_body_at(text: &str, line: u32) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut depth = 0i32;
    for (index, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if start.is_none() && depth <= 0 && type_decl(trimmed) {
            start = Some(index);
        }
        depth += trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;
        if let Some(begin) = start {
            if depth <= 0 {
                spans.push((begin, index));
                start = None;
            }
        }
    }
    let cursor = line.saturating_sub(1) as usize;
    let (begin, end) = spans.iter().find(|(begin, end)| *begin <= cursor && cursor <= *end).or_else(|| spans.first())?;
    class_body(&lines[*begin..=*end].join("\n"))
}

/// 参数：`text` 为源码全文。
/// 核心流程：定位第一个类型声明 → 只保留花括号深度为 1 的行（方法体内的局部变量不算字段）；
/// 没有类型声明时返回 None。
fn class_body(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut depth = 0i32;
    let mut inside = false;
    for line in text.lines() {
        if !inside && type_decl(line) {
            inside = true;
        }
        if inside && depth == 1 {
            out.push_str(line);
            out.push('\n');
        }
        depth += line.chars().filter(|c| *c == '{').count() as i32 - line.chars().filter(|c| *c == '}').count() as i32;
        if inside && depth <= 0 {
            inside = false;
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

/// 参数：`body` 为类体文本，`lookup` 按类名取类文本。
/// 核心流程：沿 `extends X` 逐层向上取父类文本（最多 8 层，名字重复即止），
/// 返回「直接父类 → 更上层」的顺序，供 Bulk Value 把最顶层字段排在最前。
fn parent_chain(body: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut chain = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut current = body.to_string();
    while chain.len() < 8 {
        let Some(at) = current.find("extends ") else { break };
        let rest = &current[at + "extends ".len()..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect();
        let name = name.rsplit('.').next().unwrap_or(&name).to_string();
        if name.is_empty() || !seen.insert(name.clone()) {
            break;
        }
        let Some(text) = lookup(&name) else { break };
        current = text.clone();
        chain.push(text);
    }
    chain
}

/// 参数：`line` 为一行源码。
/// 核心流程：出现独立单词 class/interface/enum/record 视为类型声明行。
fn type_decl(line: &str) -> bool {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| matches!(word, "class" | "interface" | "enum" | "record"))
}

/// 参数：`ctx` 为命令调用上下文。
/// 核心流程：取编辑器全文与该方法的映射 → `params::parse` 解析参数 → 用项目文件做类查表 →
/// `params::query_string` 拼 `k=v&k=v`；没有参数时复制空串并说明。
fn copy_query(ctx: &Invocation) -> Result<String, Error> {
    let (text, endpoint) = current_method(ctx)?;
    let list = params_at(&text, endpoint.line).ok_or("找不到方法参数列表")?;
    let project = ctx.project();
    let query = params::query_string(&params::parse(&list), |name| class_text(project.as_ref(), name));
    let note = copy_text(&query)?;
    Ok(if query.is_empty() {
        format!("{note}: no query parameters (empty string copied)")
    } else {
        format!("{note}: {query}")
    })
}

/// 参数：`ctx` 为命令调用上下文。
/// 核心流程：取方法的参数列表 → 找 `@RequestBody` 参数（只取第一个）→ 按其类型
/// `json_gen::json_for` 生成骨架并复制；没有 body 参数时报可读错误、不动剪贴板。
fn copy_body(ctx: &Invocation) -> Result<String, Error> {
    let (text, endpoint) = current_method(ctx)?;
    let list = params_at(&text, endpoint.line).ok_or("找不到方法参数列表")?;
    let body = params::parse(&list).into_iter().find(|param| param.body).ok_or("该接口没有 @RequestBody")?;
    let project = ctx.project();
    let json = json_gen::json_for_with(&body.ty, |name| class_text(project.as_ref(), name), json_options());
    let note = copy_text(&json)?;
    Ok(format!("{note}: {} body", body.ty))
}

/// 参数：`ctx` 为命令调用上下文。
/// 核心流程：取编辑器全文与「光标所在方法」的映射（没有就以文件首个映射兜底），
/// 供 copy-full-url / query / body / send 复用；缺编辑器或无映射报可读错误。
fn current_method(ctx: &Invocation) -> Result<(String, scan::Endpoint), Error> {
    let editor = ctx.editor().ok_or("请在 Java 编辑器中运行：当前没有可读取的文件")?;
    let text = editor.document()?.text;
    let line = cursor_line(&editor);
    let endpoint = pick_endpoint(scan::scan_any(&editor.path(), &text), line).ok_or(tr("No route at the caret"))?;
    Ok((text, endpoint))
}

/// 参数：`editor` 为聚焦编辑器。
/// 核心流程：取第一个光标所在行（1 基）；读不到选择时给 1，让「首个映射」兜底。
fn cursor_line(editor: &TextEditor) -> u32 {
    editor.selections().ok().and_then(|ranges| ranges.first().map(|range| range.start.line + 1)).unwrap_or(1)
}

/// 参数：`endpoints` 为文件内的端点，`line` 为光标行（1 基）。
/// 核心流程：取注解行不晚于光标行的最后一条（即光标所在方法）；都不满足时回落第一条。
fn pick_endpoint(endpoints: Vec<scan::Endpoint>, line: u32) -> Option<scan::Endpoint> {
    let chosen = endpoints.iter().rfind(|endpoint| endpoint.line <= line).cloned();
    chosen.or_else(|| endpoints.into_iter().next())
}

/// 参数：`ctx` 为调用上下文。
/// 核心流程：`ui.pick` 列出三个复制动作（US-E3-04 的聚合入口），选中项转调对应实现；
/// 选中项不合契约时报可读错误，绝不 panic。
fn pick_command(ctx: &Invocation) -> Result<String, Error> {
    let items = [
        PickItem::new(CMD_COPY_FULL_URL, &tr("Restful Toolkit: Copy Full URL")),
        PickItem::new(CMD_COPY_QUERY_PARAM, &tr("Restful Toolkit: Copy Query Param")),
        PickItem::new(CMD_COPY_REQUEST_BODY, &tr("Restful Toolkit: Copy RequestBody JSON")),
    ];
    let Some(choice) = host::pick(&tr("Restful Toolkit"), &items)? else { return Err(Error::Cancelled) };
    match choice.as_str() {
        CMD_COPY_QUERY_PARAM => copy_query(ctx),
        CMD_COPY_REQUEST_BODY => copy_body(ctx),
        CMD_COPY_FULL_URL => {
            let (_, full_url) = resolve(ctx)?;
            let note = copy_text(&full_url)?;
            Ok(format!("{note} {full_url}"))
        }
        other => Err(Error::Invalid(format!("unknown pick {other:?}"))),
    }
}


/// 参数：`route` 为一条路由。
/// 核心流程：label 形如 `GET  /api/users/{id}  — getUser`（方法在前并对齐、路径居中），
/// description 放文件名；key 用路由唯一 id，选中后据此反查路由。
fn goto_item(route: &index::Route, subs: &[String]) -> PickItem {
    let file = route.file.rsplit('/').next().unwrap_or(&route.file);
    let label = match tree::subproject_of(&route.file, subs) {
        Some(sub) => format!("[{sub}] {:<4} {}  — {}", route.method, route.path, route.handler),
        None => format!("{:<4} {}  — {}", route.method, route.path, route.handler),
    };
    PickItem::new(&tree::route_key(route), &label).description(file)
}

/// 参数：`index` 为路由索引。
/// 核心流程：按 path（再按 method、handler）排序 → 只留前 `GOTO_LIMIT` 条 →
/// 返回（候选、是否截断、总条数），候选条不会超过上限。
fn goto_candidates(index: &index::RouteIndex) -> (Vec<&index::Route>, bool, usize) {
    let mut matched: Vec<&index::Route> = index.routes.iter().collect();
    matched.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)).then_with(|| a.handler.cmp(&b.handler)));
    let total = matched.len();
    let truncated = total > GOTO_LIMIT;
    (matched.into_iter().take(GOTO_LIMIT).collect(), truncated, total)
}


/// 参数：`ctx` 为调用上下文。
/// 核心流程：按扩展名选扫描器解析当前文件，返回「路由条数 / 未解析条数」，供设置页按钮展示。
fn test_parse(ctx: &Invocation) -> Result<String, Error> {
    let editor = ctx.editor().ok_or(tr("No route at the caret"))?;
    let text = editor.document()?.text;
    let endpoints = scan::scan_any(&editor.path(), &text);
    let unresolved = endpoints.iter().filter(|endpoint| endpoint.unresolved).count();
    Ok(tr_fmt("Parsed {} routes ({} unresolved)", &[&endpoints.len().to_string(), &unresolved.to_string()]))
}

/// 参数：`text` 为源码全文，`line` 为注解所在的 1 基行号。
/// 核心流程：跳过该行起连续的注解行 → 找方法声明的 `(` → 按括号配对截出参数列表原文。
pub(crate) fn params_at(text: &str, line: u32) -> Option<String> {
    let offset: usize = text.lines().take(line.saturating_sub(1) as usize).map(|l| l.len() + 1).sum();
    let mut rest = &text[offset.min(text.len())..];
    while rest.trim_start().starts_with('@') {
        let trimmed = rest.trim_start();
        rest = &trimmed[trimmed.find('\n')? + 1..];
    }
    let tail = &rest[rest.find('(')? + 1..];
    let mut depth = 0usize;
    for (i, ch) in tail.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(tail[..i].to_string());
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// 参数：`project` 为当前项目，`name` 为类名。
/// 核心流程：按 `**/{name}.java` 找文件 → 读第一个命中的全文；没有项目或读不到返回 None。
pub(crate) fn class_text(project: Option<&Project>, name: &str) -> Option<String> {
    let project = project?;
    let paths = project.find_files(&format!("**/{name}.java"), 4).ok()?;
    paths.first().and_then(|path| project.read_file(path).ok())
}

/// 参数：`ctx` 为命令调用上下文，`key` 为树节点的 key。
/// 核心流程：按 key 前缀筛路由（root 全部、m| 模块、c| 文件、r| 单条）→
/// 每条拼完整 URL → 以 `\n\n` 拼接复制；节点下没有路由时报可读错误。
fn copy_node_urls(ctx: &Invocation, key: &str) -> Result<String, Error> {
    let index = load_index_for(project_root(ctx).as_deref());
    let routes: Vec<&index::Route> = index
        .routes
        .iter()
        .filter(|route| match key.split('|').next() {
            Some("root") => true,
            Some("m") => key.split_once('|').is_some_and(|(_, module)| tree::module_of(&route.file) == module),
            Some("c") => key.split_once('|').is_some_and(|(_, file)| route.file == file),
            Some("r") => tree::route_key(route) == key,
            _ => false,
        })
        .collect();
    if routes.is_empty() {
        return Err(Error::Failed(tr("Index not built yet")));
    }
    let base = match ctx.project() {
        Some(project) => base_url(&project)?,
        None => default_base_url(),
    };
    let urls: Vec<String> = routes.iter().map(|route| url::join_url(&base, "", &route.path)).collect();
    let joined = urls.join("\n\n");
    let note = copy_text(&joined)?;
    Ok(format!("{note}: {} URLs", urls.len()))
}

/// 参数：`ctx` 为命令调用上下文。
/// 核心流程：把索引里的方法节点做成 `pick-many` 选项 → 选中的 URL 按 `\n\n` 拼接复制
/// （宿主的树是单选，这里用多选对话框做等价落地）；没有选择时按取消处理。
fn copy_many_urls(ctx: &Invocation) -> Result<String, Error> {
    let index = load_index_for(project_root(ctx).as_deref());
    if index.routes.is_empty() {
        return Err(Error::Failed(tr("Index not built yet")));
    }
    let items: Vec<PickItem> = index
        .routes
        .iter()
        .map(|route| {
            PickItem::new(&tree::route_key(route), &format!("[{}] {}", route.method, route.handler))
                .description(&format!("{} {}", route.method, route.path))
        })
        .collect();
    let Some(keys) = host::pick_many("选择要复制的路由", &items)? else {
        return Err(Error::Cancelled);
    };
    let base = match ctx.project() {
        Some(project) => base_url(&project)?,
        None => default_base_url(),
    };
    let urls: Vec<String> = index
        .routes
        .iter()
        .filter(|route| keys.contains(&tree::route_key(route)))
        .map(|route| url::join_url(&base, "", &route.path))
        .collect();
    if urls.is_empty() {
        return Err(Error::Failed(tr("Copy failed")));
    }
    let note = copy_text(&urls.join("\n\n"))?;
    Ok(format!("{note}: {} URLs", urls.len()))
}

/// 参数：`ctx` 为命令调用上下文。
/// 核心流程：取聚焦编辑器全文 → `scan::scan_file` 找映射方法 → 类级路径取该方法上方
/// 最近的 `@RequestMapping` → 与 `base_url` 一起交给 `url::join_url` 拼装。
fn resolve(ctx: &Invocation) -> Result<(scan::Endpoint, String), Error> {
    let (text, endpoint) = current_method(ctx)?;
    let base = match ctx.project() {
        Some(project) => base_url(&project)?,
        None => default_base_url(),
    };
    let base_path = index::class_path_above(&text, endpoint.line);
    let full_url = url::join_url(&base, &base_path, &endpoint.path);
    Ok((endpoint, full_url))
}

/// 参数：`ctx` 为命令调用上下文。
/// 核心流程：收 `**/*.java` 与 `**/*.kt` → 逐个读文本（读不到就跳过）→
/// `index::RouteIndex::from_files` 建索引 → JSON 写入 storage；缺项目时报可读错误。
fn refresh_index(ctx: &Invocation, trigger: &str) -> Result<(index::RouteIndex, String, bool), Error> {
    let (mut diag, started) = begin_diagnostics(trigger, ctx);
    let outcome = refresh_index_inner(ctx, &mut diag, started);
    match &outcome {
        Ok((index, ..)) => {
            diag.routes = index.routes.len();
            checkpoint(&mut diag, started, "done");
        }
        Err(error) => {
            // 失败时保留 diag.phase 指向的失败阶段，只补错误文本。
            diag.error = Some(error.to_string());
            diag.duration_ms = now_ms().saturating_sub(started);
            let path = write_diagnostics(&diag);
            log_diagnostics(&diag, &path);
        }
    }
    outcome
}

/// 参数：`ctx` 为命令上下文，`diag` 为逐步填充的诊断快照。
/// 核心流程：项目根 → 两个 glob 的 find_files（记数与错误）→ 逐文件 read_file（记成功/失败）
/// → 增量 diff 后写 storage；任何一步失败都在 `diag.error` 里留下错误文本再返回。
fn refresh_index_inner(
    ctx: &Invocation,
    diag: &mut Diagnostics,
    started: u64,
) -> Result<(index::RouteIndex, String, bool), Error> {
    let project = match ctx.project() {
        Some(project) => project,
        None => {
            let error = Error::Failed("未拿到项目根（window 未就绪或项目正在索引），请稍后重试".to_string());
            diag.error = Some(error.to_string());
            checkpoint(diag, started, "project");
            return Err(error);
        }
    };
    diag.project_root = project.root().ok();
    // 解析器版本变了：旧索引、签名表、前缀图一起作废，本次必然全量重解析。
    diag.invalidated = invalidate_derived(diag.project_root.as_deref());
    checkpoint(diag, started, "project");
    checkpoint(diag, started, "task_begin");
    let task = host::begin_task("Restful Toolkit: 重建路由索引", true);
    checkpoint(diag, started, "task_ready");
    let mut paths = Vec::new();
    for glob in SOURCE_GLOBS {
        let began = now_ms();
        match project.find_files(glob, FILE_CAP) {
            Ok(found) => {
                if found.len() as u32 >= FILE_CAP {
                    diag.hit_cap = true;
                }
                diag.found.insert((*glob).to_string(), found.len());
                paths.extend(found);
            }
            Err(error) => diag.find_errors.push(format!("{glob}: {error}")),
        }
        diag.globs_ms.insert((*glob).to_string(), now_ms().saturating_sub(began));
        let phase = if glob.ends_with(".java") { "find_java" } else { "find_kt" };
        checkpoint(diag, started, phase);
    }
    paths.sort();
    paths.dedup();
    let total = paths.len() as u32;
    task.report(0, Some(total), Some("正在扫描…"));
    let root = diag.project_root.clone();
    let mut signatures = load_signatures_for(root.as_deref());
    let mut index = load_index_for(root.as_deref());
    let mut changed: Vec<(String, String)> = Vec::new();
    let mut py_texts: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (done, path) in paths.iter().enumerate() {
        if task.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if is_excluded_file(path) {
            diag.excluded_tests += 1;
            continue;
        }
        if !matches_project_prefix(path) {
            diag.prefix_skipped += 1;
            continue;
        }
        match project.read_file(path) {
            Ok(text) => {
                diag.files_read += 1;
                let sig = index::signature(&text);
                seen.push(path.clone());
                if signatures.get(path).copied() == Some(sig) {
                    continue;
                }
                if path.ends_with(".py") {
                    diag.websocket_skipped += scan::websocket_decorators(&text);
                    py_texts.push((path.clone(), text.clone()));
                }
                if looks_like_route_source(&text) {
                    // 签名留到真正解析完（或分块落盘）时再写，避免「记了签名没记路由」。
                    changed.push((path.clone(), text));
                } else {
                    signatures.insert(path.clone(), sig);
                    index.remove_file(path);
                    diag.files_skipped += 1;
                }
            }
            Err(error) => {
                diag.files_failed += 1;
                if diag.read_errors.len() < 5 {
                    diag.read_errors.push(ReadError { path: path.clone(), error: error.to_string() });
                }
            }
        }
        if done % 64 == 0 {
            task.report(done as u32, Some(total), Some("正在扫描…"));
        }
        if done > 0 && done % 256 == 0 {
            // 每 256 个文件落一次盘：真 trap 时文件会停在这里，能看出读到第几个。
            checkpoint(diag, started, "read");
        }
    }
    diag.files_to_parse = changed.len();
    checkpoint(diag, started, "diff");
    let gone: Vec<String> = signatures.keys().filter(|path| !seen.contains(path)).cloned().collect();
    let mut removed = 0;
    for path in gone {
        signatures.remove(&path);
        removed += index.remove_file(&path);
    }
    diag.parse_chunk = PARSE_CHUNK;
    // 建索引时构建前缀图（跨文件 include_router 链），并随索引落盘。
    let graph = fastapi::build(&py_texts);
    let _ = save_prefix_graph(root.as_deref(), &graph);
    checkpoint(diag, started, "graph");
    storage::set(&scoped_key(PENDING_KEY, root.as_deref()), "1").ok();
    checkpoint(diag, started, "parse_begin");
    let scanner = |path: &str, text: &str| -> Vec<scan::Endpoint> {
        if path.ends_with(".py") {
            scan::scan_fastapi_with(text, Some(&|receiver: &str| graph.prefix_of(path, receiver)))
        } else {
            scan::scan_any(path, text)
        }
    };
    let rescanned = parse_in_chunks(
        &mut index,
        &task,
        &changed,
        &mut signatures,
        root.as_deref(),
        diag,
        started,
        &scanner,
    )?;
    diag.rescanned = rescanned;
    diag.removed = removed;
    checkpoint(diag, started, "save");
    let cached = save_signatures_for(root.as_deref(), &signatures)?
        && match save_index_for(root.as_deref(), &index) {
            Ok(()) => true,
            Err(Error::Denied(_)) => false,
            Err(other) => return Err(other),
        };
    diag.storage_written = cached;
    if cached {
        let _ = storage::delete(&scoped_key(PENDING_KEY, root.as_deref()));
    }
    let note = if cached {
        String::new()
    } else {
        format!(
            "（{} storage：{}；{}）",
            tr("Permission missing"),
            tr("Valid for this session only"),
            tr("Open Settings > Plugins › Restful Toolkit and allow")
        )
    };
    let cap_note = cap_warning(diag.hit_cap);
    let excluded_note = if diag.excluded_tests > 0 {
        format!("；{}", tr_fmt("Excluded {} test files", &[&diag.excluded_tests.to_string()]))
    } else {
        String::new()
    };
    let message = format!(
        "扫描 {} 文件 / 命中 {} 路由 / 诊断已写入 plugins-dir（rescanned {rescanned}, removed {removed}）{cap_note}{excluded_note}{note}",
        diag.files_read,
        index.routes.len()
    );
    Ok((index, message, cached))
}

/// 参数：无。
/// 核心流程：给出带两个源码 glob 的空诊断快照。
impl Diagnostics {
    /// 参数：`trigger` 为本次刷新的来源（view/command/event/startup），`root` 为 `ctx.project()` 的根。
    /// 核心流程：填好版本、来源、根与 globs，其余计数由调用方边跑边填。
    fn new(trigger: &str, root: Option<String>) -> Self {
        Self {
            plugin_version: env!("CARGO_PKG_VERSION").to_string(),
            parser_version: PARSER_VERSION.to_string(),
            trigger: trigger.to_string(),
            phase: "start".to_string(),
            project_root: root.clone(),
            invocation_root: root,
            globs: SOURCE_GLOBS.iter().map(|glob| (*glob).to_string()).collect(),
            ..Self::default()
        }
    }
}

/// 参数：`trigger` 为来源（view/command/event/startup），`ctx` 为上下文。
/// 核心流程：一进来就写一份「刚开始」的诊断（`error` 为 null）并打同级日志，
/// 这样即使后面提前返回，也能在 diagnostics.json 里看到插件确实跑到了这一步。
fn begin_diagnostics(trigger: &str, ctx: &Invocation) -> (Diagnostics, u64) {
    let diag = Diagnostics::new(trigger, project_root(ctx));
    let path = write_diagnostics(&diag);
    log_diagnostics(&diag, &path);
    (diag, now_ms())
}

/// 分块解析的大小：每 200 个文件一组，组前续预算、组后落盘。
const PARSE_CHUNK: usize = 200;

/// 只有文本里出现这些注解之一的文件才值得解析（廉价预筛，跳过绝大多数 java 文件）。
const ROUTE_MARKERS: [&str; 7] = [
    "@RequestMapping",
    "@GetMapping",
    "@PostMapping",
    "@PutMapping",
    "@DeleteMapping",
    "@PatchMapping",
    "@Path",
];

/// 参数：`text` 为文件全文。
/// 核心流程：文本里出现任一映射注解才认为可能含路由；纯 DTO/工具类直接跳过。
fn looks_like_route_source(text: &str) -> bool {
    ROUTE_MARKERS.iter().any(|marker| text.contains(marker)) || looks_like_fastapi(text)
}

/// 参数：`text` 为文件全文。
/// 核心流程：含 `APIRouter(` 或有 `@<ident>.<verb>(` 装饰器才可能是 FastAPI 路由文件。
fn looks_like_fastapi(text: &str) -> bool {
    scan::looks_like_fastapi(text)
}

/// 解析器版本：改变解析行为时自增，旧索引会被整份丢弃并全量重解析。
pub const PARSER_VERSION: &str = "2";

/// 落盘格式：`{"parser_version": "2", "routes": [...]}`。
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredIndex {
    #[serde(default)]
    parser_version: String,
    #[serde(default)]
    routes: Vec<index::Route>,
}

/// 前缀图的落盘格式，同样带解析器版本。
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredGraph {
    #[serde(default)]
    parser_version: String,
    #[serde(default)]
    graph: fastapi::PrefixGraph,
}

/// FastAPI 前缀图的键基名（随索引一起落盘，视图/弹窗不重复解析）。
const PREFIX_GRAPH_KEY: &str = "restful.route-index.prefixes";

/// 「上次没扫完」的标记键基名：留着它，下次视图/命令触发会接着干。
const PENDING_KEY: &str = "restful.route-index.pending";

/// 参数：`index` 为待更新的索引，`task` 为当前任务，`changed` 为要解析的（路径，文本），
/// `diag`/`started` 为诊断状态。
/// 核心流程：每 `PARSE_CHUNK` 个文件一组——组前 `task.report` 续预算并检查取消，
/// 组内 `update_file`，组后写一次 `phase=parse` 诊断（带 parsed/total）；取消时提前返回。
#[allow(clippy::too_many_arguments)]
fn parse_in_chunks(
    index: &mut index::RouteIndex,
    task: &openidea_plugin::Task,
    changed: &[(String, String)],
    signatures: &mut BTreeMap<String, u64>,
    root: Option<&str>,
    diag: &mut Diagnostics,
    started: u64,
    scanner: &dyn Fn(&str, &str) -> Vec<scan::Endpoint>,
) -> Result<usize, Error> {
    let total = changed.len();
    let mut parsed = 0usize;
    for chunk in changed.chunks(PARSE_CHUNK) {
        if task.is_cancelled() {
            diag.error = Some("cancelled while parsing".to_string());
            checkpoint(diag, started, "parse");
            return Err(Error::Cancelled);
        }
        task.report(parsed as u32, Some(total as u32), Some("解析中…"));
        for (path, text) in chunk {
            index.update_file_scanned(path, text, scanner(path, text));
            signatures.insert(path.clone(), index::signature(text));
            parsed += 1;
        }
        // 每块都落盘：被 5 s 掐断时下次只处理还没写签名的文件，几次触发即可收敛。
        let saved = save_signatures_for(root, signatures).unwrap_or(false) && save_index_for(root, index).is_ok();
        diag.saved = saved;
        diag.storage_written = saved;
        diag.parsed = parsed;
        diag.files_to_parse = total;
        diag.routes = index.routes.len();
        checkpoint(diag, started, "parse");
    }
    Ok(parsed)
}

/// 参数：`diag` 为诊断快照，`started` 为开始时刻（毫秒），`phase` 为刚结束的阶段名。
/// 核心流程：把阶段名写进 `diag.phase` → 更新耗时 → 覆盖写 diagnostics.json 并打日志。
/// 每个阶段都调一次，用来判断 wasm 到底停在哪一步（trap 时文件就停在最后的阶段）。
fn checkpoint(diag: &mut Diagnostics, started: u64, phase: &str) {
    diag.phase = phase.to_string();
    diag.duration_ms = now_ms().saturating_sub(started);
    let path = write_diagnostics(diag);
    log_diagnostics(diag, &path);
}

/// 参数：`diag` 为诊断快照，`started` 为开始时刻（毫秒）。
/// 核心流程：补上耗时 → 覆盖写 diagnostics.json → 打同级日志。
fn finish_diagnostics(mut diag: Diagnostics, started: u64) {
    diag.duration_ms = now_ms().saturating_sub(started);
    let path = write_diagnostics(&diag);
    log_diagnostics(&diag, &path);
}

/// 参数：`diag` 为诊断快照，`path` 为落盘路径。
/// 核心流程：把关键字段打成一条 `host::log`，与 diagnostics.json 对齐时间线。
fn log_diagnostics(diag: &Diagnostics, path: &str) {
    host::log(
        LogLevel::Info,
        &format!(
            "diagnostics: trigger={} version={} invocation_root={:?} files={} read_failed={} routes={} storage_written={} error={:?} duration_ms={} path={path}",
            diag.trigger,
            diag.plugin_version,
            diag.invocation_root,
            diag.files_read,
            diag.files_failed,
            diag.routes,
            diag.storage_written,
            diag.error,
            diag.duration_ms
        ),
    );
}

/// 参数：`diag` 为本次刷新的诊断快照。
/// 核心流程：写到 `plugin_dir()/diagnostics.json`（插件私目录，无需 fs 授权）并返回路径；
/// 写失败时返回带原因的说明文本，绝不 panic。
fn write_diagnostics(diag: &Diagnostics) -> String {
    let Ok(dir) = host::fs::plugin_dir() else { return "<no plugin dir>".to_string() };
    let path = format!("{dir}/diagnostics.json");
    let json = serde_json::to_string_pretty(diag).unwrap_or_else(|_| "{}".to_string());
    match host::fs::write_file(&path, &json) {
        Ok(()) => path,
        Err(error) => format!("{path} (write failed: {error})"),
    }
}

/// 去抖窗口：500 ms 内的文件变更合并成一批。
const DEBOUNCE_MS: u64 = 500;

/// 参数：`hit_cap` 为是否有 glob 恰好取到 `FILE_CAP` 条。
/// 核心流程：命中上限时给一句「可能不完整」的提示；否则空串。
fn cap_warning(hit_cap: bool) -> String {
    if hit_cap {
        format!("；{}", tr_fmt("hit the {} file cap, the index may be incomplete", &[&FILE_CAP.to_string()]))
    } else {
        String::new()
    }
}

/// 参数：无。
/// 核心流程：当前墙上时间（毫秒）；拿不到时钟时返回 0（此时退化为「每批立即扫」）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 本插件扫描的来源 glob。
const SOURCE_GLOBS: [&str; 3] = ["**/*.java", "**/*.kt", "**/*.py"];

/// `refresh-index` 的诊断快照，原样写成 `diagnostics.json` 便于线上排查。
#[derive(Default, serde::Serialize)]
struct Diagnostics {
    plugin_version: String,
    trigger: String,
    /// 当前阶段：start / project / task_begin / task_ready / find_java / find_kt / read / diff / save / done。
    phase: String,
    /// 每个 glob 各自的耗时（毫秒）。
    globs_ms: BTreeMap<String, u64>,
    /// 分块解析的块大小（便于排查）。
    parse_chunk: usize,
    project_root: Option<String>,
    invocation_root: Option<String>,
    duration_ms: u64,
    globs: Vec<String>,
    found: BTreeMap<String, usize>,
    find_errors: Vec<String>,
    files_read: usize,
    files_failed: usize,
    read_errors: Vec<ReadError>,
    files_to_parse: usize,
    /// 已经解析完的文件数（分块解析的进度）。
    parsed: usize,
    /// 预筛跳过的文件数（不含映射注解）。
    files_skipped: usize,
    /// 最近一次分块落盘是否成功。
    saved: bool,
    /// 跳过但计数的 FastAPI websocket 装饰器个数。
    websocket_skipped: usize,
    /// 按 excludeGlobs 排除的测试文件个数。
    excluded_tests: usize,
    /// 按 projectPrefixFilter 跳过的文件个数。
    prefix_skipped: usize,
    /// 当前解析器版本。
    parser_version: String,
    /// 是否因解析器版本不符而整份重建。
    invalidated: bool,
    routes: usize,
    rescanned: usize,
    removed: usize,
    storage_written: bool,
    hit_cap: bool,
    error: Option<String>,
}

/// 读文件失败的一条记录（只留前 5 条）。
#[derive(serde::Serialize)]
struct ReadError {
    path: String,
    error: String,
}


/// 参数：`root` 为项目根。
/// 核心流程：取该项目的 JSON 文本解析成「路径 → 哈希」；缺失、损坏或无权限时给空表（等于全量重扫）。
fn load_signatures_for(root: Option<&str>) -> BTreeMap<String, u64> {
    storage::get(&scoped_key(SIGNATURE_KEY, root))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// 参数：`root` 为项目根，`signatures` 为最新签名表。
/// 核心流程：序列化成 JSON 写入 storage；没有 storage 权限时静默跳过（下次仍全量重扫）。
fn save_signatures_for(root: Option<&str>, signatures: &BTreeMap<String, u64>) -> Result<bool, Error> {
    let json = serde_json::to_string(signatures).map_err(|error| Error::Failed(format!("签名表序列化失败：{error}")))?;
    match storage::set(&scoped_key(SIGNATURE_KEY, root), &json) {
        Ok(()) => Ok(true),
        Err(Error::Denied(_)) => Ok(false),
        Err(other) => Err(other),
    }
}

/// 参数：`project` 为当前项目。
/// 核心流程：按 glob 收集 bootstrap* 与 application* 配置 → 读出激活 profile →
/// 基础配置先应用、`application-<profile>.*` 最后应用以覆盖 → 取端口与 context-path。
pub(crate) fn base_url(project: &Project) -> Result<String, Error> {
    let mut paths = Vec::new();
    for glob in [
        "**/bootstrap*.properties",
        "**/bootstrap*.yml",
        "**/application*.properties",
        "**/application*.yml",
        "**/application*.yaml",
    ] {
        paths.extend(project.find_files(glob, 32)?);
    }
    paths.sort();
    paths.dedup();
    let texts: Vec<(String, String)> =
        paths.iter().filter_map(|path| project.read_file(path).ok().map(|text| (path.clone(), text))).collect();
    let profile = setting_text("activeProfile")
        .or_else(|| texts.iter().find_map(|(_, text)| config::read_active_profile(text)));
    // 优先级（后者覆盖前者）：bootstrap < application < application-<profile>；同层 properties 覆盖 yml。
    let mut ranked: Vec<((u8, u8, u8), &str)> =
        texts.iter().map(|(path, text)| (rank_of(path, profile.as_deref()), text.as_str())).collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    let mut port = None;
    let mut context = None;
    for (_, text) in ranked {
        if let Some(port_value) = config::read_server_port(text) {
            port = Some(port_value);
        }
        if let Some(path_value) = config::read_context_path(text) {
            context = Some(path_value);
        }
    }
    // 配置一个字都没提到端口与 context-path 时，才轮到设置里的 baseUrl。
    if port.is_none() && context.is_none() {
        return Ok(default_base_url());
    }
    Ok(compose_base(port.unwrap_or(DEFAULT_PORT), context.as_deref()))
}

/// 参数：`key` 为英文原文键。
/// 核心流程：按 `host::locale()` 选语言包（`zh*` 走中文，其它走英文）→ 查表；
/// 缺 key 回落英文原文，再缺就返回键本身，不 panic。
pub(crate) fn tr(key: &str) -> String {
    let tag = host::locale().to_lowercase();
    let table = if tag.starts_with("zh") { LOCALE_ZH } else { LOCALE_EN };
    catalog(table).get(key).cloned().or_else(|| catalog(LOCALE_EN).get(key).cloned()).unwrap_or_else(|| key.to_string())
}

/// 参数：`key` 为带 `{}` 占位的文案键，`args` 依次填入。
/// 核心流程：取本地化模板后顺序替换 `{}`；参数不足时保留剩余空位，不 panic。
pub(crate) fn tr_fmt(key: &str, args: &[&str]) -> String {
    let mut out = tr(key);
    for arg in args {
        if let Some(at) = out.find("{}") {
            out.replace_range(at..at + 2, arg);
        }
    }
    out
}

/// 参数：`json` 为语言包文本。
/// 核心流程：解析成「键 → 文案」表；解析失败给空表，于是全部回落英文原文。
fn catalog(json: &str) -> BTreeMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// 参数：`key` 为布尔设置键，`fallback` 为缺省值。
/// 核心流程：读设置文本（已去 JSON 引号）→ `true`/`false`，其余回落 `fallback`。
fn setting_bool(key: &str, fallback: bool) -> bool {
    match setting_text(key).as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => fallback,
    }
}

/// 默认排除的测试路径（settings.excludeGlobs 未配置时用）。
pub const DEFAULT_EXCLUDE_GLOBS: &str = "**/tests/**,**/test/**,**/test_*.py,**/*_test.py,**/*Test.java,**/*Tests.java,**/src/test/**";

/// 参数：无（读 `excludeTests`）。
/// 核心流程：缺省开启；显式 `false` 才关闭。
pub(crate) fn exclude_tests_enabled() -> bool {
    setting_bool("excludeTests", true)
}

/// 参数：无（读 `excludeGlobs`）。
/// 核心流程：逗号或换行分隔，去空白去空项；未配置时用内置默认列表。
pub(crate) fn exclude_globs() -> Vec<String> {
    let raw = setting_text("excludeGlobs").unwrap_or_else(|| DEFAULT_EXCLUDE_GLOBS.to_string());
    raw.split([',', '\n']).map(str::trim).filter(|glob| !glob.is_empty()).map(str::to_string).collect()
}

/// 参数：`path` 为文件路径。
/// 核心流程：`excludeTests` 关闭时不排除；否则任一 glob 命中即排除（在建索引阶段过滤）。
pub(crate) fn is_excluded_file(path: &str) -> bool {
    exclude_tests_enabled() && exclude_globs().iter().any(|glob| glob_match(glob, path))
}

/// 参数：`pattern` 为 glob，`path` 为文件路径。
/// 核心流程：`**` 跨目录、`*` 限于单段、`?` 单字符；线性回溯，不递归爆栈。
pub(crate) fn glob_match(pattern: &str, path: &str) -> bool {
    fn go(pattern: &[u8], path: &[u8]) -> bool {
        match pattern {
            [] => path.is_empty(),
            [b'*', b'*', b'/', rest @ ..] => go(rest, path) || (!path.is_empty() && go(pattern, &path[1..])),
            [b'*', b'*', rest @ ..] => (0..=path.len()).any(|i| go(rest, &path[i..])),
            [b'*', rest @ ..] => {
                (0..=path.len()).take_while(|&i| i == 0 || path[i - 1] != b'/').any(|i| go(rest, &path[i..]))
            }
            [b'?', rest @ ..] => !path.is_empty() && path[0] != b'/' && go(rest, &path[1..]),
            [byte, rest @ ..] => path.first() == Some(byte) && go(rest, &path[1..]),
        }
    }
    go(pattern.as_bytes(), path.as_bytes())
}

/// 参数：`path` 为文件路径。
/// 核心流程：`projectPrefixFilter` 为空时一律通过；非空时只放行路径包含它的文件。
pub(crate) fn matches_project_prefix(path: &str) -> bool {
    match project_prefix() {
        Some(prefix) => path.contains(&prefix),
        None => true,
    }
}

/// 参数：无（读 `projectPrefixFilter`）。
/// 核心流程：非空时只保留文件路径包含该子串的路由；空串/未配置返回 None（不过滤）。
pub(crate) fn project_prefix() -> Option<String> {
    setting_text("projectPrefixFilter").map(|prefix| prefix.trim().to_string()).filter(|prefix| !prefix.is_empty())
}

/// 参数：无（读 `maxDepth` 与 `skipSerialVersionUid`）。
/// 核心流程：缺省 `MAX_DEPTH` 与 `true`；`maxDepth` 至少 1，非法值回落缺省。
pub(crate) fn json_options() -> json_gen::Options {
    json_gen::Options {
        max_depth: setting_u32("maxDepth", json_gen::MAX_DEPTH as u32).max(1) as usize,
        skip_serial: setting_bool("skipSerialVersionUid", true),
    }
}

/// 参数：无（读 `lineEnding`）。
/// 核心流程：`lf` → `\n`；缺省与其它值 → `\r\n`（US-E5-03 的内置默认）。
pub(crate) fn line_ending() -> String {
    if setting_text("lineEnding").as_deref() == Some("lf") { "\n".to_string() } else { "\r\n".to_string() }
}

/// 参数：`text` 为要复制的文本。
/// 核心流程：≤900 KiB 且剪贴板可用时直接写剪贴板；超限或 `clipboard` 被拒绝时降级为
/// 「写入 `fs.plugin_dir/out/` 并把文件路径复制到剪贴板」，返回可读的结果文案，不 panic。
fn copy_text(text: &str) -> Result<String, Error> {
    let fits = format::for_clipboard(text).is_ok();
    if fits {
        match copy_to_clipboard(text) {
            Ok(()) => return Ok(tr("Copied")),
            Err(Error::Denied(_)) => {}
            Err(other) => return Err(other),
        }
    }
    let dir = host::fs::plugin_dir().map_err(denied_hint)?;
    let path = format!("{dir}/restful-copy.txt");
    host::fs::write_file(&path, text).map_err(denied_hint)?;
    let copied_path = copy_to_clipboard(&path).is_ok();
    let reason = if fits { tr("Permission missing") } else { tr("Content too large") };
    let state = if copied_path { tr("Copied") } else { tr("Copy failed") };
    Ok(format!("{reason}: {path} ({state})"))
}

/// 参数：`error` 为宿主错误。
/// 核心流程：`Denied` 补上缺的权限名与 Settings 路径，其余原样返回。
fn denied_hint(error: Error) -> Error {
    match error {
        Error::Denied(cap) => Error::Failed(format!(
            "{}：{cap}（{}）",
            tr("Permission missing"),
            tr("Open Settings > Plugins > Restful Toolkit and allow")
        )),
        other => other,
    }
}

/// 参数：`key` 为设置键（`[contributes.settings]` 的 schema 键）。
/// 核心流程：`host::setting` 读当前项目/全局值（JSON 文本）→ 是字符串就解开 JSON 引号与转义，
/// 否则原样；每次调用现读，不缓存到任何静态变量。
fn setting_text(key: &str) -> Option<String> {
    let raw = host::setting(key)?;
    let raw = raw.trim();
    let value = match raw.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        Some(inner) => unescape(inner),
        None => raw.to_string(),
    };
    (!value.is_empty()).then_some(value)
}

/// 参数：`text` 为 JSON 字符串的字面内容（不含首尾引号）。
/// 核心流程：解开 `\\n`、`\\t`、`\\r`、`\\"`、`\\\\` 这类转义，其余字符原样保留。
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// 参数：`key` 为整数设置键，`fallback` 为缺省值。
/// 核心流程：读设置文本 → 按 u32 解析；缺失或解析失败都退回 `fallback`。
fn setting_u32(key: &str, fallback: u32) -> u32 {
    setting_text(key).and_then(|text| text.parse().ok()).unwrap_or(fallback)
}

/// 参数：无（读 `extraHeaders` 设置）。
/// 核心流程：按行拆 `Name: Value`，跳过空行与没有冒号的行；每次现读，不缓存。
fn extra_headers() -> Vec<(String, String)> {
    setting_text("extraHeaders")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

/// 参数：无（读 `baseUrl` 设置）。
/// 核心流程：有设置用设置，否则 `DEFAULT_BASE_URL`；给拿不到配置文件的路径与没有项目的窗口用。
pub(crate) fn default_base_url() -> String {
    setting_text("baseUrl").unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// 参数：`path` 为配置文件路径，`profile` 为当前激活 profile。
/// 核心流程：返回（application 优先于 bootstrap、是否 profile 专属、是否 properties）
/// 三元组，升序应用后写覆盖先写。
fn rank_of(path: &str, profile: Option<&str>) -> (u8, u8, u8) {
    let name = path.rsplit('/').next().unwrap_or(path);
    let is_application = u8::from(!name.starts_with("bootstrap"));
    let for_profile = u8::from(profile.is_some_and(|profile| name.contains(&format!("application-{profile}."))));
    let properties = u8::from(name.ends_with(".properties"));
    (is_application, for_profile, properties)
}

/// 参数：`port` 为端口，`context_path` 为配置里的 context-path 原文。
/// 核心流程：拼 `http://localhost:<port>`，context-path 归一化后仅当不是根路径才追加。
fn compose_base(port: u16, context_path: Option<&str>) -> String {
    let mut base = format!("http://localhost:{port}");
    let context = context_path.map(url::normalize_path).unwrap_or_default();
    if context != "/" {
        base.push_str(&context);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;
    use openidea_plugin::views::Views;

    /// 当前 mock 项目的根。
    fn root_of(host: &MockHost) -> String {
        host.invocation().project().and_then(|project| project.root().ok()).unwrap_or_default()
    }

    /// 当前项目的索引。
    fn current_index(host: &MockHost) -> index::RouteIndex {
        crate::load_index_for(Some(&root_of(host)))
    }

    /// 当前项目的索引存储键。
    fn index_key_of(host: &MockHost) -> String {
        crate::scoped_key(crate::INDEX_KEY, Some(&root_of(host)))
    }

    /// 剪贴板里 URL 部分（缺路径变量时后面会附提示，用空行分隔）。
    fn url_of(host: &MockHost) -> String {
        host.clipboard().unwrap_or_default().split("\n\n").next().unwrap_or_default().to_string()
    }

    const CONTROLLER: &str = "\
package demo;

@RestController
@RequestMapping(\"/api\")
public class UserController {
    @GetMapping(\"/v1/users/{id}\")
    public User get(@PathVariable Long id) {
        return null;
    }
}
";

    #[test]
    fn copies_the_full_url_of_the_mapping() {
        let host = MockHost::install();
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        let message = RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert!(message.is_some(), "成功时应回报一条消息");
        assert_eq!(url_of(&host), "http://localhost:8080/api/v1/users/{id}");
    }

    #[test]
    fn base_url_comes_from_the_config_files() {
        let host = MockHost::install();
        host.add_file("application.yml", "server:\n  port: 8081\n  servlet:\n    context-path: /gateway\n");
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:8081/gateway/api/v1/users/{id}");
    }

    #[test]
    fn profile_file_overrides_the_base_config() {
        let host = MockHost::install();
        host.add_file("application.properties", "server.port=8081\nspring.profiles.active=dev\n");
        host.add_file("application-dev.properties", "server.port=9090\n");
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:9090/api/v1/users/{id}");
    }

    #[test]
    fn no_mapping_is_a_readable_error_and_keeps_the_clipboard() {
        let host = MockHost::install();
        host.open_editor("/project/Plain.java", "java", "class Plain {\n    int value = 1;\n}\n", &[]);
        let err = RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap_err();
        assert!(err.to_string().contains(&tr("No route at the caret")), "错误文本要能读：{err}");
        assert_eq!(host.clipboard(), None, "解析失败不得写入剪贴板");
    }

    #[test]
    fn refresh_index_writes_the_storage_key() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        let message = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("扫描 1 文件 / 命中 1 路由"), "{message}");
        let stored = host.storage().get(&index_key_of(&host)).cloned().unwrap_or_default();
        assert!(stored.contains("/api/v1/users/{id}"), "{stored}");
    }

    #[test]
    fn refresh_index_rescans_only_changed_files_and_starts_a_task() {
        let host = MockHost::install();
        host.add_file("src/A.java", CONTROLLER);
        host.add_file("src/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/others"));
        let first = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert!(first.contains("rescanned 2"), "{first}");
        assert_eq!(host.tasks().len(), 1, "刷新要开 task 续预算");
        host.add_file("src/A.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/users"));
        let second = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert!(second.contains("rescanned 1"), "{second}");
        assert!(second.contains("命中 2 路由"), "{second}");
        let paths: Vec<String> = current_index(&host).routes.iter().map(|route| route.path.clone()).collect();
        assert!(paths.contains(&"/api/v1/users".to_string()), "{paths:?}");
        assert!(paths.contains(&"/api/v1/others".to_string()), "{paths:?}");
    }

    #[test]
    fn send_request_uses_the_endpoint_params_as_query_and_body() {
        let host = MockHost::install().with_capabilities(&["net:localhost", "project.read", "editor.read"]);
        host.add_file(
            "src/UserCreateReq.java",
            "public class UserCreateReq {\n    private String name;\n}\n",
        );
        host.open_editor("/project/OrderController.java", "java", ORDER_CONTROLLER, &[]);
        host.on_fetch(|request| {
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, "http://localhost:8080/api/v1/orders?name=&age=0");
            let body = String::from_utf8_lossy(request.body.as_deref().unwrap_or_default()).into_owned();
            assert_eq!(body, "{\n  \"name\": \"demoData\"\n}");
            Ok(openidea_plugin::HttpResponse { status: 201, headers: Vec::new(), body: Vec::new() })
        });
        RestfulToolkit::default().run_command(CMD_SEND_REQUEST, &host.invocation(), None).unwrap();
        assert_eq!(host.requests().len(), 1);
    }

    #[test]
    fn send_request_applies_path_vars_and_appends_the_rest_as_query() {
        let host = MockHost::install().with_capabilities(&["net:localhost"]);
        host.on_fetch(|request| {
            assert_eq!(request.url, "http://localhost:8080/api/users/42?name=alice");
            assert_eq!(request.method, "GET");
            assert!(request.body.is_none(), "GET 不带 body");
            Ok(openidea_plugin::HttpResponse { status: 200, headers: Vec::new(), body: Vec::new() })
        });
        RestfulToolkit::default()
            .run_command(CMD_SEND_REQUEST, &host.invocation(), Some("GET http://localhost:8080/api/users/{id} id=42 name=alice"))
            .unwrap();
    }

    #[test]
    fn send_request_keeps_path_vars_without_a_value() {
        let host = MockHost::install().with_capabilities(&["net:localhost"]);
        host.on_fetch(|request| {
            assert_eq!(request.url, "http://localhost:8080/api/users/42/orders/{oid}");
            Ok(openidea_plugin::HttpResponse { status: 200, headers: Vec::new(), body: Vec::new() })
        });
        RestfulToolkit::default()
            .run_command(
                CMD_SEND_REQUEST,
                &host.invocation(),
                Some("GET http://localhost:8080/api/users/{id}/orders/{oid} id=42"),
            )
            .unwrap();
    }

    #[test]
    fn search_finds_routes_by_fragment_wildcard_and_full_url() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        let page = plugin.search("routes", "users", 10, &host.invocation()).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title, "GET /api/v1/users/{id}");
        let SearchTarget::Location(location) = &page.items[0].target else { panic!("应可跳转") };
        assert_eq!(location.range.start.line, 5, "注解行 6 → 0 基 5");
        assert_eq!(plugin.search("routes", "/api/v1/*", 10, &host.invocation()).unwrap().items.len(), 1);
        assert_eq!(
            plugin.search("routes", "http://10.0.3.7:8080/api/v1/users/42", 10, &host.invocation()).unwrap().items.len(),
            1,
            "完整 URL 去掉 scheme/host/port 后按前缀命中"
        );
        assert!(plugin.search("routes", "nothing", 10, &host.invocation()).unwrap().items.is_empty());
    }

    #[test]
    fn copy_many_urls_joins_the_chosen_routes_with_a_blank_line() {
        let host = MockHost::install();
        host.add_file("src/A.java", CONTROLLER);
        host.add_file("src/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/others"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let keys: Vec<String> = current_index(&host).routes.iter().map(tree::route_key).collect();
        let answer: Vec<&str> = keys.iter().map(String::as_str).collect();
        host.answer_next_prompt(Some(&answer));
        RestfulToolkit::default().run_command(CMD_COPY_MANY_URLS, &host.invocation(), None).unwrap();
        let copied = host.clipboard().unwrap_or_default();
        assert_eq!(copied, "http://localhost:8080/api/v1/users/{id}\n\nhttp://localhost:8080/api/v1/others", "{copied}");
    }

    #[test]
    fn copy_full_url_appends_the_unresolved_hint() {
        let host = MockHost::install();
        host.open_editor(
            "/project/U.java",
            "java",
            "@RestController\npublic class U {\n    @GetMapping(ApiConst.USERS)\n    public void users() {\n    }\n}\n",
            &[],
        );
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        let copied = host.clipboard().unwrap_or_default();
        assert!(copied.starts_with("http://localhost:8080/ApiConst.USERS"), "{copied}");
        assert!(copied.contains(&tr("Path not statically resolved")), "提示要附在 URL 后面：{copied}");
    }

    #[test]
    fn properties_override_yml_in_the_same_tier() {
        let host = MockHost::install();
        host.add_file("application.yml", "server.port: 8081\n");
        host.add_file("application.properties", "server.port=8082\n");
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:8082/api/v1/users/{id}");
    }

    #[test]
    fn profile_files_override_the_base_application_files() {
        let host = MockHost::install();
        host.add_file("application.properties", "server.port=8081\n");
        host.add_file("application-dev.yml", "server.port: 9090\n");
        host.set_setting("activeProfile", "\"dev\"");
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:9090/api/v1/users/{id}");
    }

    #[test]
    fn settings_active_profile_wins_over_the_file() {
        let host = MockHost::install();
        host.add_file("application-dev.yml", "server.port: 9090\n");
        host.add_file("application-prod.yml", "server.port: 7070\n");
        host.add_file("application.yml", "spring:\n  profiles:\n    active: dev\n");
        host.set_setting("activeProfile", "\"prod\"");
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:7070/api/v1/users/{id}");
    }

    #[test]
    fn search_routes_filters_by_url_fragment() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let found = RestfulToolkit::default().run_command(CMD_SEARCH_ROUTES, &host.invocation(), Some("users"))
            .unwrap()
            .unwrap();
        assert!(found.contains("GET /api/v1/users/{id}"), "{found}");
        let missing = RestfulToolkit::default().run_command(CMD_SEARCH_ROUTES, &host.invocation(), Some("nothing-here"))
            .unwrap_err();
        assert!(missing.to_string().contains("没有匹配"), "{missing}");
    }

    #[test]
    fn endpoints_come_from_the_storage_cache() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let endpoints = RestfulToolkit::default().endpoints(&host.invocation()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].method.as_deref(), Some("GET"));
        assert_eq!(endpoints[0].path, "/api/v1/users/{id}");
        assert_eq!(endpoints[0].location.path, "/project/src/UserController.java");
        assert_eq!(endpoints[0].location.range.start.line, 5, "注解行 6 → 0 基 5");
    }

    const ORDER_CONTROLLER: &str = "\
package demo;

@RestController
@RequestMapping(\"/api\")
public class OrderController {
    @PostMapping(\"/v1/orders\")
    public Order create(String name, int age, @RequestBody UserCreateReq req) {
        return null;
    }
}
";

    #[test]
    fn copies_the_query_param_skeleton() {
        let host = MockHost::install();
        host.open_editor("/project/OrderController.java", "java", ORDER_CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_QUERY_PARAM, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("name=&age=0"), "US-E3-02：数值 0、字符串空串");
    }

    #[test]
    fn copies_the_request_body_json() {
        let host = MockHost::install();
        host.add_file(
            "src/UserCreateReq.java",
            "public class UserCreateReq {\n    private String name;\n    private int age;\n}\n",
        );
        host.open_editor("/project/OrderController.java", "java", ORDER_CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_REQUEST_BODY, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("{\n  \"name\": \"demoData\",\n  \"age\": 1\n}"));
    }

    #[test]
    fn missing_request_body_is_a_readable_error() {
        let host = MockHost::install();
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        let err = RestfulToolkit::default().run_command(CMD_COPY_REQUEST_BODY, &host.invocation(), None).unwrap_err();
        assert!(err.to_string().contains("@RequestBody"), "{err}");
        assert_eq!(host.clipboard(), None);
    }

    #[test]
    fn copy_url_works_from_a_tree_node_key() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let key = tree::route_key(&current_index(&host).routes[0]);
        RestfulToolkit::default().run_command(CMD_COPY_URL, &host.invocation(), Some(&key)).unwrap();
        assert_eq!(url_of(&host), "http://localhost:8080/api/v1/users/{id}");
    }

    #[test]
    fn send_request_renders_the_response_in_the_details_pane() {
        let host = MockHost::install().with_capabilities(&["net:localhost"]);
        host.on_fetch(|request| {
            assert_eq!(request.method, "GET");
            Ok(openidea_plugin::HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: br#"{"a":1}"#.to_vec(),
            })
        });
        let mut plugin = RestfulToolkit::default();
        let message = plugin
            .run_command(CMD_SEND_REQUEST, &host.invocation(), Some("GET http://localhost:8080/api/users/1"))
            .unwrap()
            .unwrap();
        assert!(message.contains("200"), "{message}");
        assert_eq!(host.requests().len(), 1);
        let tabs = plugin.detail(view::VIEW_ID, "root", &host.invocation()).unwrap();
        let response = tabs.iter().find(|tab| tab.id == "response").expect("应有 Response tab");
        let Pane::Text(text) = &response.body else { panic!("响应应是只读文本") };
        assert!(text.contains("状态：200"), "{text}");
        assert!(text.contains("\"a\": 1"), "body 应已美化：{text}");
    }

    #[test]
    fn extra_headers_become_request_headers() {
        let host = MockHost::install().with_capabilities(&["net:localhost"]);
        host.set_setting("extraHeaders", "\"Cookie: sid=1\\nX-Trace: abc\"");
        host.on_fetch(|request| {
            assert_eq!(request.headers, vec![("Cookie".to_string(), "sid=1".to_string()), ("X-Trace".to_string(), "abc".to_string())]);
            Ok(openidea_plugin::HttpResponse { status: 200, headers: Vec::new(), body: Vec::new() })
        });
        RestfulToolkit::default()
            .run_command(CMD_SEND_REQUEST, &host.invocation(), Some("http://localhost:8080/api"))
            .unwrap();
    }

    #[test]
    fn settings_supply_the_base_url_and_the_timeout() {
        let host = MockHost::install()
            .with_capabilities(&["net:localhost", "project.read", "editor.read", "storage", "clipboard"]);
        host.set_setting("baseUrl", "\"http://api.example:9000\"");
        host.set_setting("requestTimeoutMs", "1234");
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://api.example:9000/api/v1/users/{id}");
        host.on_fetch(|request| {
            assert_eq!(request.timeout_ms, Some(1234), "超时应来自设置");
            Ok(openidea_plugin::HttpResponse { status: 204, headers: Vec::new(), body: Vec::new() })
        });
        RestfulToolkit::default()
            .run_command(CMD_SEND_REQUEST, &host.invocation(), Some("http://localhost:8080/api"))
            .unwrap();
    }

    #[test]
    fn send_request_without_net_permission_is_a_readable_error() {
        let host = MockHost::install();
        let mut plugin = RestfulToolkit::default();
        let error = plugin
            .run_command(CMD_SEND_REQUEST, &host.invocation(), Some("http://localhost:8080/api/users/1"))
            .unwrap_err();
        assert!(error.to_string().contains(&tr("Request failed")), "{error}");
        assert!(error.to_string().contains("net"), "{error}");
        let tabs = plugin.detail(view::VIEW_ID, "root", &host.invocation()).unwrap();
        assert!(tabs.iter().all(|tab| tab.id != "response"), "没有响应就不该有 Response tab");
        assert!(matches!(tabs[0].body, Pane::Message(_)));
    }

    #[test]
    fn converts_the_class_body_to_json_compressed_and_bulk() {
        let host = MockHost::install();
        let text = "package demo;\n\npublic class User {\n    private String name;\n    private int age;\n\n    public String greet() {\n        String local = \"x\";\n        return local;\n    }\n}\n";
        host.open_editor("/project/User.java", "java", text, &[]);
        RestfulToolkit::default().run_command(CMD_CONVERT_JSON, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("{\n  \"name\": \"demoData\",\n  \"age\": 1\n}"));
        RestfulToolkit::default().run_command(CMD_CONVERT_JSON_COMPRESSED, &host.invocation(), None).unwrap();
        let compressed = host.clipboard().unwrap_or_default();
        assert!(!compressed.contains('\n'), "压缩版必须单行：{compressed}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&compressed).unwrap(),
            serde_json::from_str::<serde_json::Value>(r#"{"name":"demoData","age":1}"#).unwrap()
        );
        RestfulToolkit::default().run_command(CMD_CONVERT_BULK_VALUE, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("name:demoData\r\nage:1"), "US-E5-03：缺省行尾 CRLF");
    }

    #[test]
    fn convert_without_a_class_is_a_readable_error() {
        let host = MockHost::install();
        host.open_editor("/project/notes.txt", "text", "just text\n", &[]);
        let err = RestfulToolkit::default().run_command(CMD_CONVERT_JSON, &host.invocation(), None).unwrap_err();
        assert!(err.to_string().contains("类"), "{err}");
        assert_eq!(host.clipboard(), None);
    }

    #[test]
    fn details_are_cached_per_node_key() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let key = tree::route_key(&current_index(&host).routes[0]);
        let mut plugin = RestfulToolkit::default();
        let first = plugin.detail(view::VIEW_ID, &key, &host.invocation()).unwrap();
        let other = plugin.detail(view::VIEW_ID, "root", &host.invocation()).unwrap();
        assert_eq!(first.iter().map(|tab| tab.label.as_str()).collect::<Vec<_>>(), ["Method", "Full URL", "Params", "Body"]);
        assert!(matches!(other[0].body, Pane::Message(_)), "根节点只有提示");
        assert_eq!(plugin.details.len(), 2, "两个 key 各占一格缓存，互不覆盖");
        let again = plugin.detail(view::VIEW_ID, &key, &host.invocation()).unwrap();
        assert_eq!(again, first, "命中缓存后内容一致");
    }

    #[test]
    fn max_depth_and_skip_serial_settings_take_effect() {
        let host = MockHost::install();
        host.add_file("src/B.java", "public class B {\n    int v;\n}\n");
        host.add_file("src/A.java", "public class A {\n    B b;\n}\n");
        host.add_file("src/S.java", "public class S {\n    static final long serialVersionUID = 1L;\n    int n;\n}\n");
        host.open_editor("/project/A.java", "java", "public class A {\n    B b;\n}\n", &[]);
        host.set_setting("maxDepth", "1");
        RestfulToolkit::default().run_command(CMD_CONVERT_JSON, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("{\n  \"b\": {}\n}"), "maxDepth=1 时子对象截断");

        host.open_editor("/project/S.java", "java", "public class S {\n    static final long serialVersionUID = 1L;\n    int n;\n}\n", &[]);
        RestfulToolkit::default().run_command(CMD_CONVERT_JSON, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("{\n  \"n\": 1\n}"), "默认跳过 serialVersionUID");
        host.set_setting("skipSerialVersionUid", "false");
        RestfulToolkit::default().run_command(CMD_CONVERT_JSON, &host.invocation(), None).unwrap();
        let json = host.clipboard().unwrap_or_default();
        assert!(json.contains("serialVersionUID"), "关掉跳过项后应保留：{json}");
    }

    #[test]
    fn line_ending_setting_changes_bulk_value() {
        let host = MockHost::install();
        let text = "public class U {\n    int id;\n    String name;\n}\n";
        host.open_editor("/project/U.java", "java", text, &[]);
        RestfulToolkit::default().run_command(CMD_CONVERT_BULK_VALUE, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("id:1\r\nname:demoData"));
        host.set_setting("lineEnding", "\"lf\"");
        RestfulToolkit::default().run_command(CMD_CONVERT_BULK_VALUE, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("id:1\nname:demoData"));
    }

    #[test]
    fn runtime_text_follows_the_locale_and_falls_back_to_english() {
        let host = MockHost::install();
        host.set_locale("zh-CN");
        assert_eq!(tr("Copied"), "已复制");
        host.set_locale("fr");
        assert_eq!(tr("Copied"), "Copied", "缺语言包回落英文");
        assert_eq!(tr("no such key"), "no such key", "缺 key 回落键本身");
    }

    #[test]
    fn denied_clipboard_falls_back_to_a_file_instead_of_failing() {
        let host = MockHost::install().with_capabilities(&["project.read", "editor.read", "storage"]);
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        let message = RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("restful-copy.txt"), "降级到文件并把路径写进结果：{message}");
        assert_eq!(host.clipboard(), None, "剪贴板被拒时不再写入");
    }

    #[test]
    fn oversized_copies_go_to_a_file_and_the_path_is_copied() {
        let host = MockHost::install();
        let big = "x".repeat(format::CLIPBOARD_SAFE_LIMIT + 16);
        let message = copy_text(&big).unwrap();
        assert!(message.contains("restful-copy.txt"), "{message}");
        let copied = host.clipboard().unwrap_or_default();
        assert!(copied.ends_with("restful-copy.txt"), "剪贴板里是路径：{copied}");
        assert!(format::for_clipboard(&big).is_err(), "900 KiB 以上不该走剪贴板正文");
    }

    #[test]
    fn endpoints_report_the_jaxrs_framework_and_stop_at_ten_thousand() {
        let host = MockHost::install();
        host.add_file("src/UserResource.java", "@Path(\"/api\")\npublic class UserResource {\n    @GET\n    @Path(\"/users/{id}\")\n    public User get(Long id) {\n        return null;\n    }\n}\n");
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let endpoints = RestfulToolkit::default().endpoints(&host.invocation()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].framework, "jaxrs", "JAX-RS 来源要标 jaxrs");

        let mut index = current_index(&host);
        let sample = index.routes[0].clone();
        index.routes = (0..ENDPOINT_LIMIT + 5).map(|i| { let mut route = sample.clone(); route.path = format!("/p/{i}"); route }).collect();
        let (capped, truncated) = endpoints_of(&index);
        assert!(truncated, "超过 1 万条要报告截断");
        assert_eq!(capped.len(), ENDPOINT_LIMIT);
    }

    #[test]
    fn file_change_events_rescan_only_the_touched_file() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        host.add_file("src/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/others"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        host.add_file("src/UserController.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/members"));
        let event = Event::FilesChanged(openidea_plugin::FilesChanged {
            created: Vec::new(),
            modified: vec!["/project/src/UserController.java".into()],
            removed: Vec::new(),
        });
        // 测试里不等 500 ms：last_flush=0 让这一批直接放行。
        let mut plugin = RestfulToolkit { last_flush: 0, ..RestfulToolkit::default() };
        plugin.handle_events(&[event], &host.invocation());
        let paths: Vec<String> = current_index(&host).routes.iter().map(|route| route.path.clone()).collect();
        assert!(paths.contains(&"/api/v1/members".to_string()), "{paths:?}");
        assert!(paths.contains(&"/api/v1/others".to_string()), "其他文件不受影响：{paths:?}");

        let removed = Event::FilesChanged(openidea_plugin::FilesChanged {
            created: Vec::new(),
            modified: Vec::new(),
            removed: vec!["/project/src/B.java".into()],
        });
        plugin.last_flush = 0;
        plugin.handle_events(&[removed], &host.invocation());
        assert!(current_index(&host).routes.iter().all(|route| route.file != "/project/src/B.java"), "删除的文件要出索引");
    }

    #[test]
    fn kotlin_files_work_with_the_same_commands() {
        let host = MockHost::install();
        let kt = "@RestController\n@RequestMapping(\"/k\")\nclass KController {\n    @PostMapping(\"/items\")\n    fun create(): String { return \"\" }\n}\n";
        host.open_editor("/project/K.kt", "kotlin", kt, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:8080/k/items");
    }

    #[test]
    fn bulk_value_walks_the_whole_extends_chain() {
        let host = MockHost::install();
        host.add_file("src/Root.java", "public class Root {\n    long tenantId;\n}\n");
        host.add_file("src/Base.java", "public class Base extends Root {\n    int id;\n}\n");
        let child = "public class Child extends Base {\n    String name;\n}\n";
        host.open_editor("/project/Child.java", "java", child, &[]);
        RestfulToolkit::default().run_command(CMD_CONVERT_BULK_VALUE, &host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("tenantId:1\r\nid:1\r\nname:demoData"));
    }

    #[test]
    fn search_strips_the_configured_context_path() {
        let host = MockHost::install();
        host.add_file("application.properties", "server.servlet.context-path=/gateway\n");
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        let page = plugin
            .search("routes", "http://10.0.3.7:8080/gateway/api/v1/users/42", 10, &host.invocation())
            .unwrap();
        assert_eq!(page.items.len(), 1, "剥掉 context-path 后应命中");
    }

    #[test]
    fn denied_storage_keeps_the_index_in_memory_for_the_session() {
        let host = MockHost::install().with_capabilities(&["project.read", "editor.read"]);
        host.add_file("src/UserController.java", CONTROLLER);
        let mut plugin = RestfulToolkit::default();
        let message = plugin.run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("扫描 1 文件 / 命中 1 路由"), "{message}");
        assert!(message.contains(&tr("Permission missing")), "不能静默失败：{message}");
        assert!(message.contains(&tr("Valid for this session only")), "要说明只在本次会话有效：{message}");
        assert_eq!(plugin.endpoints(&host.invocation()).unwrap().len(), 1, "没落盘也要能在本次会话用");
    }

    #[test]
    fn refresh_index_writes_a_step_by_step_diagnostics_json() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        let message = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("诊断已写入 plugins-dir"), "{message}");

        let dir = openidea_plugin::host::fs::plugin_dir().expect("mock 有插件目录");
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).expect("诊断文件应存在");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["project_root"], "/project");
        assert_eq!(value["globs"].as_array().map(Vec::len), Some(3), "java/kt/py 三个 glob");
        assert_eq!(value["found"]["**/*.java"], 1);
        assert_eq!(value["files_read"], 1, "读成功数");
        assert_eq!(value["files_failed"], 0, "读失败数");
        assert_eq!(value["read_errors"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["files_to_parse"], 1);
        assert_eq!(value["routes"], 1);
        assert_eq!(value["storage_written"], true);
        assert!(value["error"].is_null(), "成功时没有错误文本");
    }

    #[test]
    fn indexes_are_namespaced_per_project() {
        let host = MockHost::install();
        host.add_file("/project/src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let key_a = scoped_key(INDEX_KEY, Some("/project"));
        assert!(host.storage().contains_key(&key_a), "按项目根命名空间落盘：{key_a}");
        assert_eq!(load_index_for(Some("/project")).routes.len(), 1);

        host.set_project_root(Some("/other"));
        assert!(load_index_for(Some("/other")).routes.is_empty(), "新项目不会读到别人的索引");
        host.add_file("/other/src/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/others"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let key_b = scoped_key(INDEX_KEY, Some("/other"));
        assert_ne!(key_a, key_b, "两个项目各一把键");
        assert_eq!(load_index_for(Some("/project")).routes.len(), 1, "A 的索引不被 B 覆盖");
        let b_paths: Vec<String> = load_index_for(Some("/other")).routes.iter().map(|route| route.path.clone()).collect();
        assert_eq!(b_paths, ["/api/v1/others"]);
    }

    #[test]
    fn switching_projects_does_not_show_the_other_projects_routes() {
        let host = MockHost::install();
        host.add_file("/project/src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        host.set_project_root(Some("/fresh"));
        let mut plugin = RestfulToolkit { verify_pending: false, ..RestfulToolkit::default() };
        let view = plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        let openidea_plugin::views::Body::Tree(items) = view.body else { panic!("应是树") };
        assert_eq!(items.len(), 1, "没索引的项目只给空态节点，不显示别的项目路由");
        assert_eq!(items[0].label, tr("Restful Routes: 0 routes"));
        assert_eq!(load_index_for(Some("/project")).routes.len(), 1, "A 的索引仍在自己的键下");
    }

    #[test]
    fn the_file_cap_is_reported_when_it_is_hit() {
        let _host = MockHost::install(); // tr() 要读 locale，需要宿主
        assert_eq!(FILE_CAP, 20_000, "旧上限 2000 会截断大项目");
        assert!(cap_warning(true).contains("20000"), "{}", cap_warning(true));
        assert!(cap_warning(true).contains("incomplete"));
        assert!(cap_warning(false).is_empty());
    }

    #[test]
    fn diagnostics_walk_the_phases_and_stop_at_the_failing_one() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let dir = openidea_plugin::host::fs::plugin_dir().expect("mock 有插件目录");
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["phase"], "done", "扫完最后停在 done");
        let globs_ms = value["globs_ms"].as_object().expect("每个 glob 单独计时");
        assert!(globs_ms.contains_key("**/*.java") && globs_ms.contains_key("**/*.kt"), "{globs_ms:?}");
        assert!(globs_ms["**/*.java"].is_u64());

        let empty = MockHost::install();
        empty.set_project_root(None);
        let _ = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &empty.invocation(), None).unwrap_err();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["phase"], "project", "失败阶段留在 project");
        assert!(value["error"].as_str().unwrap_or_default().contains("未拿到项目根"));
    }

    #[test]
    fn parsing_is_chunked_and_renews_the_task_budget() {
        let host = MockHost::install();
        for i in 0..450 {
            host.add_file(
                &format!("src/C{i}.java"),
                &format!("@RestController\n@RequestMapping(\"/c{i}\")\npublic class C{i} {{\n    @GetMapping(\"/x\")\n    public String x() {{\n        return null;\n    }}\n}}\n"),
            );
        }
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let reports: Vec<String> = host.tasks()[0].reports.iter().filter_map(|(_, _, message)| message.clone()).collect();
        let parsing = reports.iter().filter(|message| message.contains("解析中")).count();
        assert!(parsing >= 3, "450 个文件按 200 一块应有 3 次续期：{reports:?}");
        let dir = openidea_plugin::host::fs::plugin_dir().unwrap();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["phase"], "done");
        assert_eq!(value["parsed"], 450);
        assert_eq!(value["files_to_parse"], 450);
        assert_eq!(value["parse_chunk"], 200);
    }

    #[test]
    fn files_without_mapping_annotations_are_skipped_cheaply() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        for i in 0..5 {
            host.add_file(&format!("src/Dto{i}.java"), &format!("public class Dto{i} {{\n    private int i;\n}}\n"));
        }
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let dir = openidea_plugin::host::fs::plugin_dir().unwrap();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["files_read"], 6, "六个文件都读了");
        assert_eq!(value["files_skipped"], 5, "没有注解的五个被预筛跳过");
        assert_eq!(value["files_to_parse"], 1, "只有控制器进解析");
        assert_eq!(value["routes"], 1);
    }

    #[test]
    fn a_second_run_only_parses_the_files_that_changed() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        host.add_file("src/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/others"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        host.add_file("src/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/health"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let dir = openidea_plugin::host::fs::plugin_dir().unwrap();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["files_to_parse"], 1, "第二次只处理改过的那个文件");
        assert_eq!(value["parsed"], 1);
        assert_eq!(value["saved"], true, "分块落盘标记");
        assert_eq!(value["routes"], 2);
        let paths: Vec<String> = current_index(&host).routes.iter().map(|route| route.path.clone()).collect();
        assert!(paths.contains(&"/api/v1/health".to_string()), "{paths:?}");
    }

    #[test]
    fn a_pending_marker_makes_the_view_resume_the_build() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let root = root_of(&host);
        storage::set(&scoped_key(PENDING_KEY, Some(&root)), "1").unwrap();
        let mut plugin = RestfulToolkit::default();
        plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        assert!(host.tasks().len() >= 2, "有未完成标记时视图会接着跑一次");
    }

    #[test]
    fn a_cancelled_task_stops_before_parsing() {
        let host = MockHost::install();
        let task = openidea_plugin::host::begin_task("test", true);
        host.cancel_tasks();
        let mut index = index::RouteIndex::default();
        let mut diag = Diagnostics::new("command", Some("/project".into()));
        let changed = vec![("src/A.java".to_string(), CONTROLLER.to_string())];
        let mut signatures = BTreeMap::new();
        let scanner = |path: &str, text: &str| scan::scan_any(path, text);
        let err = parse_in_chunks(
            &mut index,
            &task,
            &changed,
            &mut signatures,
            Some("/project"),
            &mut diag,
            now_ms(),
            &scanner,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Cancelled), "取消要提前返回：{err:?}");
        assert_eq!(diag.phase, "parse");
        assert!(diag.parsed == 0 && index.routes.is_empty(), "取消后不该解析任何文件");
        assert!(diag.error.as_deref().unwrap_or_default().contains("cancel"));
    }

    #[test]
    fn a_large_scan_writes_progress_beyond_256_files() {
        let host = MockHost::install();
        for i in 0..300 {
            host.add_file(&format!("src/C{i}.java"), &format!("public class C{i} {{\n}}\n"));
        }
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let dir = openidea_plugin::host::fs::plugin_dir().expect("mock 有插件目录");
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["files_read"], 300, "读满 300 个");
        assert_eq!(value["phase"], "done");
    }

    #[test]
    fn diagnostics_are_written_even_without_a_project() {
        let host = MockHost::install();
        host.set_project_root(None);
        let err = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap_err();
        assert!(err.to_string().contains("未拿到项目根"), "{err}");

        let dir = openidea_plugin::host::fs::plugin_dir().expect("mock 有插件目录");
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).expect("提前返回也要落盘");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["trigger"], "command");
        assert_eq!(value["plugin_version"], env!("CARGO_PKG_VERSION"));
        assert!(value["invocation_root"].is_null(), "没有项目时写 null");
        assert!(value["error"].as_str().unwrap_or_default().contains("未拿到项目根"), "{}", value["error"]);
        assert!(value["duration_ms"].is_u64());
    }

    const FASTAPI_APP: &str = "from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.get(\"/items\")\ndef items():\n    return []\n";

    #[test]
    fn an_old_parser_version_index_is_discarded_and_rebuilt() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        let root = root_of(&host);
        // 旧版本写的索引（parser_version=1，且带着一条假的 line=0 路由）。
        let legacy = serde_json::json!({
            "parser_version": "1",
            "routes": [{"method": "GET", "path": "/stale", "handler": "old", "line": 0,
                        "file": "/project/src/Stale.java", "unresolved": true, "framework": "spring"}]
        });
        storage::set(&scoped_key(INDEX_KEY, Some(&root)), &legacy.to_string()).unwrap();
        storage::set(&scoped_key(SIGNATURE_KEY, Some(&root)), "{\"src/UserController.java\": 1}").unwrap();
        assert!(load_index_for(Some(&root)).routes.is_empty(), "旧版本索引读出来就是空");

        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let index = current_index(&host);
        assert!(index.routes.iter().all(|route| route.path != "/stale"), "旧条目必须被丢弃");
        assert_eq!(index.routes.len(), 1, "全量重解析出真实路由");
        let dir = openidea_plugin::host::fs::plugin_dir().unwrap();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["invalidated"], true);
        assert_eq!(value["parser_version"], PARSER_VERSION);
    }

    #[test]
    fn a_current_parser_version_index_is_incremental() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let dir = openidea_plugin::host::fs::plugin_dir().unwrap();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["invalidated"], false, "同版本不重建");
        assert_eq!(value["files_to_parse"], 0, "签名没变就不重解析");
        assert_eq!(current_index(&host).routes.len(), 1);
    }

    #[test]
    fn fastapi_cross_file_prefixes_and_lines_reach_the_index() {
        let host = MockHost::install();
        host.add_file(
            "/project/pkg/api/users.py",
            "from fastapi import APIRouter\n\nrouter = APIRouter(prefix=\"/users\")\n\n@router.get(\"/{id}\")\ndef get_user(id: int):\n    return None\n",
        );
        host.add_file(
            "/project/pkg/app.py",
            "from fastapi import FastAPI\nfrom .api.users import router as users_router\n\napp = FastAPI()\napp.include_router(users_router, prefix=\"/api\")\n",
        );
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let route = current_index(&host).routes.iter().find(|route| route.file.ends_with("users.py")).cloned().expect("应有 users 路由");
        assert_eq!(route.path, "/api/users/{id}", "include_router 的 prefix 要拼上");
        assert_eq!(route.line, 5, "行号是装饰器行");
        assert_eq!(route.framework, "fastapi");
    }

    #[test]
    fn test_sources_are_excluded_at_index_time() {
        let host = MockHost::install();
        host.add_file("/project/src/UserController.java", CONTROLLER);
        host.add_file("/project/tests/test_api.py", FASTAPI_APP);
        let message = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert_eq!(current_index(&host).routes.len(), 1, "测试文件不进索引");
        assert!(message.contains(&tr_fmt("Excluded {} test files", &["1"])), "{message}");
        let dir = openidea_plugin::host::fs::plugin_dir().unwrap();
        let json = openidea_plugin::host::fs::read_file(&format!("{dir}/diagnostics.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["excluded_tests"], 1);
    }

    #[test]
    fn custom_exclude_globs_replace_the_defaults() {
        let host = MockHost::install();
        host.add_file("/project/tests/test_api.py", FASTAPI_APP);
        host.add_file("/project/src/UserController.java", CONTROLLER);
        host.set_setting("excludeGlobs", "\"**/legacy/**\"");
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let paths: Vec<String> = current_index(&host).routes.iter().map(|route| route.path.clone()).collect();
        assert!(paths.contains(&"/items".to_string()), "自定义 glob 只排除 legacy：{paths:?}");
        assert!(paths.contains(&"/api/v1/users/{id}".to_string()));

        host.add_file("/project/legacy/Old.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let files: Vec<String> = current_index(&host).routes.iter().map(|route| route.file.clone()).collect();
        assert!(files.iter().all(|file| !file.contains("/legacy/")), "{files:?}");
    }

    #[test]
    fn project_prefix_filter_keeps_only_matching_files() {
        let host = MockHost::install();
        host.add_file("/project/haiyi-ai/backend/app/a.py", FASTAPI_APP);
        host.add_file("/project/haiyi-ai-commercial/backend/app/b.py", &FASTAPI_APP.replace("/items", "/orders"));
        host.set_setting("projectPrefixFilter", "\"haiyi-ai-commercial/backend\"");
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let files: Vec<String> = current_index(&host).routes.iter().map(|route| route.file.clone()).collect();
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].contains("haiyi-ai-commercial/backend"), "{files:?}");
    }

    #[test]
    fn sub_projects_add_a_top_level_tree_layer_and_pick_prefix() {
        let host = MockHost::install();
        host.add_file("/project/haiyi-ai/backend/app/a.py", FASTAPI_APP);
        host.add_file("/project/haiyi-ai-commercial/backend/app/b.py", &FASTAPI_APP.replace("/items", "/orders"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        let openidea_plugin::views::Body::Tree(items) = view.body else { panic!("应是树") };
        let modules: Vec<&str> = items.iter().filter(|item| item.parent.as_deref() == Some(tree::ROOT)).map(|item| item.label.as_str()).collect();
        assert_eq!(modules, ["haiyi-ai", "haiyi-ai-commercial"], "顶层是子项目名");

        let index = current_index(&host);
        let files: Vec<&str> = index.routes.iter().map(|route| route.file.as_str()).collect();
        let subs = tree::subprojects(&files);
        let label = goto_item(&index.routes[0], &subs).label;
        assert!(label.starts_with("[haiyi-ai"), "候选条要带子项目前缀：{label}");
    }

    #[test]
    fn diagnostics_carry_version_trigger_root_and_duration() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let dir = openidea_plugin::host::fs::plugin_dir().expect("mock 有插件目录");
        let path = format!("{dir}/diagnostics.json");
        let json = openidea_plugin::host::fs::read_file(&path).expect("诊断文件应存在");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["plugin_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["trigger"], "command");
        assert_eq!(value["invocation_root"], "/project");
        assert_eq!(value["project_root"], "/project");
        assert!(value["duration_ms"].is_u64(), "耗时字段应是数字：{}", value["duration_ms"]);
        assert_eq!(value["routes"], 1);
        assert_eq!(value["storage_written"], true);
    }

    #[test]
    fn an_empty_index_shows_one_status_node_in_the_view() {
        let host = MockHost::install();
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        let openidea_plugin::views::Body::Tree(items) = view.body else { panic!("应是树") };
        assert_eq!(items.len(), 1, "空索引只给一个根节点");
        assert_eq!(items[0].label, tr("Restful Routes: 0 routes"));
        assert!(items[0].detail.as_deref().unwrap_or_default().contains("Rebuild Route Index"), "{:?}", items[0].detail);
    }

    #[test]
    fn endpoints_and_the_status_bar_both_express_the_empty_state() {
        let host = MockHost::install();
        let mut plugin = RestfulToolkit::default();
        assert!(plugin.endpoints(&host.invocation()).unwrap().is_empty(), "契约上只能给空表");
        let status = host.status_items().get("restful-routes").cloned().expect("空态要挂状态栏文案");
        assert_eq!(status.text, tr("Restful Routes: 0 routes"), "状态栏要说明 0 条路由");
        assert_eq!(status.command.as_deref(), Some(CMD_REFRESH_INDEX), "点它直接重建索引");
    }

    #[test]
    fn refresh_reports_the_scanned_file_and_route_counts() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        let message = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("扫描 1 文件 / 命中 1 路由"), "{message}");

        let empty = MockHost::install();
        empty.add_file("src/Plain.java", "public class Plain {\n}\n");
        let message = RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &empty.invocation(), None).unwrap().unwrap();
        assert!(message.contains("扫描 1 文件 / 命中 0 路由"), "{message}");
    }

    #[test]
    fn the_view_builds_the_index_lazily_when_empty() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        let openidea_plugin::views::Body::Tree(items) = view.body else { panic!("应是树") };
        assert!(items.len() > 1, "首帧就把索引建好并渲染出路由：{items:?}");
        assert_eq!(host.tasks().len(), 1, "懒建必须走 task");
        assert!(!current_index(&host).routes.is_empty());
    }

    #[test]
    fn the_view_skips_the_build_when_the_index_is_not_empty() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        assert_eq!(host.tasks().len(), 1, "索引非空时不再建");
    }

    #[test]
    fn file_change_events_are_debounced_and_merged() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        host.add_file("src/UserController.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/members"));
        let changed = Event::FilesChanged(openidea_plugin::FilesChanged {
            created: Vec::new(),
            modified: vec!["/project/src/UserController.java".into()],
            removed: Vec::new(),
        });
        let mut plugin = RestfulToolkit { last_flush: now_ms(), ..RestfulToolkit::default() };
        plugin.handle_events(std::slice::from_ref(&changed), &host.invocation());
        assert!(current_index(&host).routes.iter().any(|route| route.path.contains("users")), "500 ms 内先攒着不扫");
        assert_eq!(plugin.pending.len(), 1, "变更进了去抖缓冲");
        plugin.last_flush = 0;
        plugin.handle_events(&[changed], &host.invocation());
        assert!(current_index(&host).routes.iter().any(|route| route.path == "/api/v1/members"), "窗口过了合并后一起扫");
        assert!(plugin.pending.is_empty());
    }

    #[test]
    fn single_flight_refuses_a_second_build() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        let mut plugin = RestfulToolkit::default();
        plugin.building.store(true, Ordering::SeqCst);
        plugin.last_flush = 0;
        let changed = Event::FilesChanged(openidea_plugin::FilesChanged {
            created: Vec::new(),
            modified: vec!["/project/src/UserController.java".into()],
            removed: Vec::new(),
        });
        plugin.handle_events(&[changed], &host.invocation());
        plugin.ensure_index(&host.invocation());
        assert!(host.tasks().is_empty(), "已有任务在跑时既不重扫也不建");
        assert_eq!(plugin.pending.len(), 1, "变更留着等下一批");
    }

    #[test]
    fn the_view_reports_that_the_index_is_building() {
        let host = MockHost::install();
        let mut plugin = RestfulToolkit::default();
        plugin.building.store(true, Ordering::SeqCst);
        let view = plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        let openidea_plugin::views::Body::Message(text) = view.body else { panic!("构建中要表达状态") };
        assert_eq!(text, tr("Restful Routes: building the index…"));
    }

    #[test]
    fn goto_route_reports_an_empty_index() {
        let host = MockHost::install();
        let err = RestfulToolkit::default().run_command(CMD_GOTO_ROUTE, &host.invocation(), None).unwrap_err();
        assert!(err.to_string().contains("Rebuild Route Index"), "{err}");
    }

    #[test]
    fn goto_route_is_a_single_step_pick_that_opens_the_declaration() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let route = current_index(&host).routes[0].clone();
        // 只回答一次（选中的 key）：不再有输入框那一层。
        host.answer_next_prompt(Some(&[tree::route_key(&route).as_str()]));
        RestfulToolkit::default().run_command(CMD_GOTO_ROUTE, &host.invocation(), None).unwrap();
        assert_eq!(host.opened_locations(), vec![("/project/src/UserController.java".to_string(), 5, 0)],
            "注解行 6 → 0 基 5");
    }

    #[test]
    fn goto_route_rejects_keys_that_are_not_offered() {
        let host = MockHost::install();
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        host.answer_next_prompt(Some(&["r|/nope.java|1|GET|/nope"]));
        let err = RestfulToolkit::default().run_command(CMD_GOTO_ROUTE, &host.invocation(), None).unwrap_err();
        assert!(!err.to_string().is_empty(), "候选外的 key 要报错：{err}");
        assert!(host.opened_locations().is_empty());
    }

    #[test]
    fn goto_items_sort_by_path_and_format_the_label() {
        let mut index = index::RouteIndex::default();
        let route = |method: &str, path: &str, handler: &str| index::Route {
            method: method.into(),
            path: path.into(),
            handler: handler.into(),
            line: 3,
            file: "/project/src/UserController.java".into(),
            unresolved: false,
            framework: "spring".into(),
        };
        index.routes = vec![route("POST", "/api/zebra", "create"), route("GET", "/api/users/{id}", "getUser")];
        let (routes, truncated, total) = goto_candidates(&index);
        assert!(!truncated);
        assert_eq!(total, 2);
        assert_eq!(routes[0].path, "/api/users/{id}", "按 path 排序");
        let item = goto_item(routes[0], &[]);
        assert_eq!(item.label, "GET  /api/users/{id}  — getUser", "方法在前、空格对齐、路径居中");
        assert_eq!(item.description.as_deref(), Some("UserController.java"));
        assert_eq!(goto_item(routes[1], &[]).label, "POST /api/zebra  — create");
    }

    #[test]
    fn goto_candidates_truncate_at_the_cap() {
        let mut index = index::RouteIndex::default();
        index.routes = (0..GOTO_LIMIT + 5)
            .map(|i| index::Route {
                method: "GET".into(),
                path: format!("/api/users/{i:05}"),
                handler: "get".into(),
                line: 1,
                file: "/project/A.java".into(),
                unresolved: false,
                framework: "spring".into(),
            })
            .collect();
        let (routes, truncated, total) = goto_candidates(&index);
        assert_eq!(routes.len(), GOTO_LIMIT, "最多喂 2000 条");
        assert!(truncated);
        assert_eq!(total, GOTO_LIMIT + 5);
        assert_eq!(routes[0].path, "/api/users/00000", "截断前已排序");
    }

    #[test]
    fn the_aggregate_command_offers_the_three_actions() {
        let host = MockHost::install();
        host.open_editor("/project/OrderController.java", "java", ORDER_CONTROLLER, &[]);
        host.answer_next_prompt(Some(&[CMD_COPY_QUERY_PARAM]));
        let message = RestfulToolkit::default().run_command(CMD_PICK, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("name=&age=0"), "{message}");
        assert_eq!(host.clipboard().as_deref(), Some("name=&age=0"));
    }

    #[test]
    fn manifest_declares_the_shortcut_and_the_settings_action() {
        let manifest = include_str!("../plugin.toml");
        assert!(manifest.contains("key = \"Alt+Shift+R\""), "聚合命令要绑 Alt+Shift+R");
        assert!(manifest.contains("[[contributes.settings.pages.actions]]"));
        assert!(manifest.contains("command = \"test-parse\""));
    }

    #[test]
    fn the_parse_action_reports_route_counts() {
        let host = MockHost::install();
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        let message = RestfulToolkit::default().run_command(CMD_TEST_PARSE, &host.invocation(), None).unwrap().unwrap();
        assert!(message.contains("Parsed 1 routes"), "{message}");
        assert!(message.contains("0 unresolved"), "{message}");
    }

    const TWO_METHODS: &str = "\
package demo;

@RestController
@RequestMapping(\"/api\")
public class TwoController {
    @GetMapping(\"/first\")
    public String first() {
        return null;
    }

    @GetMapping(\"/second\")
    public String second() {
        return null;
    }
}
";

    #[test]
    fn commands_prefer_the_mapping_at_the_caret() {
        let host = MockHost::install();
        // 光标放在第二个方法体内（第 12 行）。
        host.open_editor("/project/TwoController.java", "java", TWO_METHODS, &[(11, 4, 11, 4)]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        assert_eq!(url_of(&host), "http://localhost:8080/api/second");
    }

    #[test]
    fn class_json_follows_the_caret_to_the_enclosing_class() {
        let host = MockHost::install();
        let text = "public class First {\n    int a;\n}\n\npublic class Second {\n    String b;\n}\n";
        host.open_editor("/project/Both.java", "java", text, &[(5, 4, 5, 4)]);
        RestfulToolkit::default().run_command(CMD_CONVERT_JSON, &host.invocation(), None).unwrap();
        let json = host.clipboard().unwrap_or_default();
        assert!(json.contains("\"b\""), "取光标所在的第二个类：{json}");
        assert!(!json.contains("\"a\""), "{json}");
    }

    #[test]
    fn missing_path_variables_get_a_readable_hint() {
        let host = MockHost::install();
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        RestfulToolkit::default().run_command(CMD_COPY_FULL_URL, &host.invocation(), None).unwrap();
        let copied = host.clipboard().unwrap_or_default();
        assert!(copied.contains(&tr_fmt("Missing path variables: {}", &["id"])), "{copied}");
        assert!(copied.starts_with("http://localhost:8080/api/v1/users/"), "URL 仍然复制：{copied}");
    }

    #[test]
    fn tree_and_detail_labels_follow_the_locale() {
        let host = MockHost::install();
        host.set_locale("zh-CN");
        host.add_file("src/UserController.java", CONTROLLER);
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        let view = plugin.render(view::VIEW_ID, None, &host.invocation()).unwrap();
        let openidea_plugin::views::Body::Tree(items) = view.body else { panic!("应是树") };
        assert_eq!(items[0].label, "共 1 个服务", "根节点计数要本地化");
        let tabs = plugin.detail(view::VIEW_ID, &tree::route_key(&current_index(&host).routes[0]), &host.invocation()).unwrap();
        let labels: Vec<&str> = tabs.iter().map(|tab| tab.label.as_str()).collect();
        assert_eq!(labels, ["方法", "完整 URL", "参数", "请求体"]);
    }

    #[test]
    fn empty_search_queries_are_grouped_by_module() {
        let host = MockHost::install();
        host.add_file("service-a/src/main/java/A.java", CONTROLLER);
        host.add_file("service-b/src/main/java/B.java", &CONTROLLER.replace("/v1/users/{id}", "/v1/zebra"));
        RestfulToolkit::default().run_command(CMD_REFRESH_INDEX, &host.invocation(), None).unwrap();
        let mut plugin = RestfulToolkit::default();
        let page = plugin.search("routes", "", 10, &host.invocation()).unwrap();
        assert_eq!(page.items[0].description.as_deref().map(|d| d.starts_with("/project/service-a/")), Some(true));
        assert_eq!(page.items.len(), 2);
    }

    #[test]
    fn unknown_command_is_reported() {
        let host = MockHost::install();
        host.open_editor("/project/UserController.java", "java", CONTROLLER, &[]);
        let err = RestfulToolkit::default().run_command("no-such-command", &host.invocation(), None).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }
}
