# 共享简报（供各分册撰写者引用，不要改动本文件）

## 目标
把 IntelliJ 插件 restful-toolkit-fix 的能力复刻为 openidea 的 WASM 插件，产出用户故事与小步交付计划。

## 宿主与插件形态
- 宿主：/root/job/ai-code/openidea（Rust + Tauri 的 IDE）
- 插件：一个目录 + `plugin.toml`（id/name/version/engines.openidea/wasm/activation/[capabilities]/[[contributes.*]]）
- 运行时：Rust SDK（sdk/rust/openidea-plugin），实现 `Plugin::run_command`，`register_plugin!` 组合 world 导出；可选导出 `views` / `endpoints` / `details` / `search`
- 各插件独立 Cargo 工程，`crate-type=["cdylib","rlib"]`，`--target wasm32-wasip2`

## 硬约束（全部已核实）
1. 无 PSI/AST：只有文件文本（16 MiB）、索引符号（class/method/field + range，最多 1000 条）、documentIndexer 回调；`textDocument/*` 命名空间被宿主禁止 → 注解解析必须自己写文本解析
2. `[[contributes.endpoints]]`：record 仅 `method?/path/framework?/handler?/location{path,range}`；`files` ≤64 个 glob；调用硬预算 2 s；≤1 万条；整批校验失败整批作废；无 module/group 字段；不产生工具窗，只进 Search Everywhere 的 Endpoints、`.http` 补全、声明行 gutter
3. `[[contributes.views]]`：树是扁平列表 + parent 键（expandable/expanded），展开触发 `expand` 事件需插件重渲染；`activate(key)` 双击/回车后插件调 `views.open_location(path,line,col)`（0 基行、UTF-16 列）；隐藏视图不渲染；无懒加载子节点 API；`detail=true` 可挂只读面板（`Pane::Text` ≤1 MiB）
4. 命令 = `plugin.<id>.<cmd>`；菜单含 `viewItem.context`（树节点右键）、`editor.context`、`tools`、`mainToolbar`；菜单/命令 `when` 支持 `editorLangId == java`；快捷键本身不能带 `when`
5. `ui.copy-to-clipboard` 静默截断 1 MiB；编辑器写入需 `selections` + `version`（stale 需重试）
6. `net.fetch`：权限按 host 声明（`net:<host>` / `net.http:<host>` / `net:ask`）；`net:ask` 只覆盖 HTTPS 与 loopback，明文 HTTP 非 loopback 必须写死 manifest；body ≤8 MiB；响应 ≤16 MiB 全量缓冲无流式；timeout 默认 30 s 且被本次调用剩余预算截断
7. 执行预算：命令/视图渲染/事件 5 s；`tasks.begin` 后可续到 60 s
8. `storage` 64 MiB KV 跨重启保留；`fs.plugin_dir` 无需授权；settings 只有读取 API（`settings.get`），写不回
9. `locales/<tag>.json` 可本地化 manifest 与视图文本；代码 `format!` 出来的字符串不会自动翻译
10. 闲置 300 s 回收，内存全丢、`activate` 重跑，状态须从 storage/索引重建
11. 单测：`openidea_plugin::mock::MockHost`（可脚本化 http、剪贴板、编辑器、find_files）

## 需改 openidea 本体的点（依赖时标注「需改本体:路径:行」）
- crates/ide-host/src/plugins/endpoints.rs:99（2 s 硬预算）
- crates/ide-plugin/src/types.rs:567-576、crates/ide-extension/src/capability.rs:155-181（net:ask 覆盖明文）
- wit/endpoints.wit:9-20（endpoint 缺 group 字段）
- src/features/http-client/generatedRequests.ts:9,45-58（插件不可调用内建 HTTP Client 动作）
- wit/io.wit:69-74（响应 8 MiB 注释 / 实现 16 MiB 不一致）

## 旧插件需复刻的功能单元（/root/job/ai-code/restful-toolkit-fix）
- U1 按 URL 搜索并跳转（Ctrl+\ / Ctrl+Alt+N，剪贴板预填、通配匹配）
- U2 Services 工具窗：模块→服务树、根节点计数、HTTP 方法图标、双击跳源码、刷新
- U3 服务详情面板：method / 完整 URL / RequestParams 键值预填 / RequestBody JSON
- U4 内置 HTTP 请求客户端：Send、`{var}` 替换、query 拼接、响应美化、Ctrl+Enter 格式化、双击复制
- U5 复制完整 URL / 相对 URL（含 server.port 与 context-path）
- U6 复制 Query Param（Key Value，跳过 @RequestBody，对象展开字段）
- U7 复制 RequestBody JSON
- U8 类 → JSON / JSON 压缩 / Bulk Value
- U9 树节点右键：Copy Full Url（多选换行）/ Jump to Source；工具栏刷新
- U10 读 application[-profile].properties|yml 的 server.port / context-path / profile
- U11 服务发现：Spring MVC（@Controller/@RestController/@RequestMapping/@{Get,Post,Put,Delete,Patch}Mapping）+ JAX-RS（@Path/@GET…）+ Kotlin，类×方法笛卡尔积拼接归一化
- U12 Popup 快捷菜单（Alt+Shift+R，聚合 U5~U7）

## 明确不继承的旧缺陷
EDT 上全量扫描重建树；后台线程改 UI；URL 通配 pattern 与 path 传反；static 可变自关联计数导致 Convert to JSON 行为漂移；请求无超时；版本号双源不一致。

## 写作约定
- 全中文；事实性结论带 `file:line`
- 每个故事固定 7 字段：编号+标题 / 故事句（作为<角色>，我希望…，以便…）/ 验收标准（3-5 条 Given-When-Then，可用 MockHost 或沙盒验证）/ 技术要点（contributes 段或 world 接口 + file:line）/ 依赖（故事编号或「需改本体:路径:行」）/ 规模（S≤1 天，M≈2-3 天，L≈3-5 天）/ 映射（U 编号）
- 不写实现代码（可写 `plugin.toml` 片段与目录树）；不出现 TODO / 待补充；故事之间不重复
- 故事编号固定：E1 US-E1-01..05；E2 US-E2-01..04；E3 US-E3-01..04；E4 US-E4-01..03；E5 US-E5-01..03；E6 US-E6-01..03
- 每个分册文件 ≤150 行，超了压缩措辞，不许省略故事
- shell 里 find/ls/sort/head/wc 有别名异常（find 实际是 fd），列目录用 python3，读文件用 view
