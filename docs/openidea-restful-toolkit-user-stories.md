# OpenIDEA 版 Restful Toolkit 复刻 · 用户故事与小步交付计划

> 目标宿主：openidea（WASM 组件插件）。旧实现：IntelliJ 插件 restful-toolkit-fix。
> 公共约束见 `_brief.md`；本文由分册合并而成。

# OpenIDEA 版 Restful Toolkit 复刻 · 第一部分：背景、角色与能力约束

> 事实基线：`/root/job/ai-code/openidea` @ `097221c1b`（2026-11-20）、`/root/job/ai-code/restful-toolkit-fix` @ `3fd3aaa`。未标路径的 `file:line` 相对 openidea；旧插件文件相对 restful-toolkit-fix。本文只裁定范围、角色与降级，故事与验收见各分册。

## 一、背景与目标

### 1.1 一句话价值主张

在 OpenIDEA 的 WASM 沙箱里重建旧 IDEA 插件「按 URL 找方法、看服务树、一键复制/发送请求」的完整闭环：没有 PSI，只有 ≤16 MiB 文件文本（`wit/fs.wit:23`）与 ≤1000 条符号索引（`wit/workspace.wit:99-105`），注解解析、配置推导、JSON 生成因此全部由插件自写文本解析器承担，交互面复用宿主已有的 views / details / endpoints / search 贡献点（`docs/en/plugins/manifest.mdx:103-130`），并顺手修掉旧插件在 EDT 上全量重建树、无超时请求等缺陷（`/root/job/ai-code/docs/plans/_brief.md:46-48`）。

### 1.2 与旧插件的对应关系

12 个功能单元（U1–U12）是唯一功能基线，下表按功能簇归并，给出旧插件落点与本次取舍；逐单元的宿主侧裁定见 §3。

| 功能簇 | U 编号 | 旧插件落点（file:line） | 复刻取舍 |
| --- | --- | --- | --- |
| 服务发现 | U11 | `resolver/SpringResolver.java:37,52-167`、`resolver/JaxrsResolver.java:24-43`、`KtClassHelper.java:19` | 保留 Spring MVC + JAX-RS + Kotlin 全量注解面与类×方法笛卡尔积，改用文本扫描（§3 U11） |
| 配置推导 | U10 | `method/action/PropertiesHandler.java:62,95-109,161-171` | 保留 port / profile / context-path 推导，补齐 Boot 2.3+ 新键（§3 U10） |
| 服务树与详情 | U2、U3、U9 | `navigator/RestServiceStructure.java:98,234`、`navigator/RestServiceDetail.java:41,84`、`src/main/resources/META-INF/plugin.xml:200-219` | 用 views + detail-provider 重建，多选与可编辑详情的损失写进 §3 U2/U3/U9 |
| 复制与快捷菜单 | U5、U6、U7、U12 | `method/action/GenerateUrlAction.java:24`、`GenerateQueryParamAction.java:21`、`GenerateQueryParamJsonAction.java:21`、`plugin.xml:113-118,129-142` | 改为编辑器命令 + 右键子菜单，Alt+Shift+R 降为默认绑定（§3 U5–U7/U12） |
| 请求客户端 | U4 | `navigator/RestServiceDetail.java:120-158`、`common/RequestHelper.java:49` | 保留 Send、`{var}` 替换、query 拼接、响应展示，传输换成 `net.fetch`（§3 U4） |
| 类 → JSON | U8 | `plugin.xml:166-177` | 保留三档输出（JSON / 压缩 / Bulk Value），字段来源受限（§3 U8） |
| URL 搜索 | U1 | `plugin.xml:186-187`、`navigation/action/GotoRequestMappingAction.java:29` | 交给 Search Everywhere 分类，旧快捷键退化为两步（§3 U1） |

### 1.3 MVP 范围

- 语言与框架：Java / Kotlin 源文件中的 Spring MVC（`@Controller`/`@RestController`/`@RequestMapping`/`@{Get,Post,Put,Delete,Patch}Mapping`）与 JAX-RS（`@Path`/`@GET` 等），只按文本与注解工作。
- 交互面：Services 位置的路由树 + 只读详情面板（`docs/en/plugins/guides/views.mdx:11,35`、`wit/details.wit:20-42`）；一组复制类编辑器命令与右键子菜单；一个能发送请求的面板；类→JSON 三条命令；一个 URL 搜索分类。
- 工程面：单插件目录 + `plugin.toml`（贡献点清单见 `docs/en/plugins/manifest.mdx:103-130`）、`locales/<tag>.json` 中英双语（`manifest.mdx:136-142`）、`MockHost` 单测（`sdk/rust/openidea-plugin/src/lib.rs:52`）、闲置 300 s 回收后从 `storage` 重建（`crates/ide-plugin/src/types.rs:454`、`wit/host.wit:187-190`）。
- 交付面：独立插件工程，`openidea-plugin link` 装入本机 IDE（`docs/en/plugins/testing.mdx:52`）；路由同时以 `[[contributes.endpoints]]` 发布进 Search Everywhere、`.http` 补全与声明行 gutter（`docs/en/plugins/guides/indexing.mdx:164-171`）。

### 1.4 非目标

1. 不做 PSI / 类型推断 / 运行时 Bean 分析：`textDocument/*` 被宿主禁止（`wit/host.wit:219-224`），注解值只认字面量与同类常量。
2. 不做完整 HTTP Client：无 cookie jar、multipart、流式响应、WebSocket（`wit/io.wit:47-74,76-85`）。
3. 不接管内建 HTTP Client 的请求生成链路，也不复制它：那是前端动作写 scratch 文件（`src/features/http-client/generatedRequests.ts:9,45-58`），插件没有等价 WIT 接口。
4. 不做 OpenAPI / Swagger 导出、mock server、压测。
5. 不追求与旧插件像素级一致：视图 body 只有 message / tree / list / form / table / html（`wit/views.wit:68-78`），没有可编辑的多行控件。
6. 不做 Gradle / Maven 模块解析，模块归属只按 `src/main/` 路径前缀推导（§3 U2）。

## 二、角色与使用场景

### 2.1 Persona A：后端开发（写接口的人）

痛点：改完 mapping 不确定最终对外 URL；本地自测要手敲 JSON 与 query。

- A1 新增 `@GetMapping("/{id}")` 后自检：光标停在方法上执行「Copy Full URL」，确认前缀没有出现 `//`，直接贴进浏览器（U11 归一化、U5）。
- A2 本地起服务自测：Services 树双击 `POST /api/orders`，详情给出 method、完整 URL、`Key: Value` 参数骨架与 `@RequestBody` JSON，点「在请求面板中打开」发一次看响应（U2、U3、U4）。
- A3 切 profile 换环境：`application-dev.yml` 里 `server.port=8081`、`server.servlet.context-path=/gateway`，复制的 URL 必须跟着变，不能永远是 8080（U10、U5）。

### 2.2 Persona B：联调 / 测试（拿别人接口的人）

痛点：手上只有一条 URL 或一张接口清单，需要反查源码并快速组请求。

- B1 只有一条 URL：从聊天窗口复制 `http://10.0.3.7:8080/api/v1/orders/42`，在 Search Everywhere 粘进去命中 `/api/v1/orders/{id}`，回车跳到声明行（U1、U11 通配匹配）。
- B2 批量取地址贴测试用例：树里逐个右键 Copy Full Url；多选不可用，改用组节点上的「Copy All URLs」拿到换行列表（U2、U9）。
- B3 造请求体与参数：DTO 类上「Convert to JSON」得骨架，方法上「Copy Query Param」得 `key: value`，一并粘进 Postman 的 Bulk Edit（U8、U6、U7）。

### 2.3 Persona C：代码走读 / 新人（读别人代码的人）

痛点：只想知道某个 URL 由谁处理、参数契约是什么，不想改任何文件。

- C1 反向找入口：按模块展开树，根节点「Found N services」先判断规模，再逐层展开到方法并双击跳声明（U2、U9）。
- C2 理解参数契约：选中节点看详情的 RequestParams 键值与 `@RequestBody` JSON，分辨哪些是 query、哪些是 path 变量、哪个是 body（U3、U6）。
- C3 就地取用：光标停在 mapping 方法上，用 Alt+Shift+R 或右键子菜单复制 URL / Query / body，全程不弹额外窗口、不写文件（U12、U5、U7）。

## 三、能力约束与降级决策

结论：12 个单元全部有落点，无一项因宿主能力缺失而放弃；但有 5 处必须降级（U1 一步搜索、U3 只读详情、U4 明文与响应展示、U9 多选、U5–U8 剪贴板 1 MiB），另有 5 处只有改宿主本体才能补齐（见文末汇总）。

| 旧能力(U编号) | openidea 侧实现方式 | 限制 | 降级方案或需改本体(file:line) |
| --- | --- | --- | --- |
| U1 按 URL 搜索跳转（Ctrl+\ / Ctrl+Alt+N，剪贴板预填、通配；`plugin.xml:186-187`） | `[[contributes.searchCategories]]` 注册 URL 分类（`docs/en/plugins/manifest.mdx:114`、`wit/search.wit:3-33`）；另给命令：`read-clipboard`（`wit/host.wit:157`）→ `input` 预填（`wit/host.wit:170-172`）→ `pick` 选结果（`wit/host.wit:160-164`） | 搜索回调在后台跑，禁止读剪贴板与弹窗（`wit/host.wit:150-157`、`wit/search.wit:31-33`）；`pick` 只有列表、没有过滤输入框（`src/workbench/inputDialogs.tsx:285-333`）；快捷键不能带 `when`（`docs/en/plugins/guides/commands.mdx:147-151`） | 主入口改为 Search Everywhere 分类（可边打边筛）；旧快捷键退化为两步对话框；`*`、`{}` 通配由插件自己匹配。要一步到位需改本体给 `pick` 加过滤框：`wit/host.wit:160-164` |
| U2 Services 树：模块→服务、根节点计数、方法图标、双击跳源码、刷新（`RestServiceStructure.java:98,234`、`plugin.xml:200-219`） | `[[contributes.views]]` `location = "services"`（`views.mdx:11,35`）渲染 `Body::Tree` 扁平列表 + `parent`（`wit/views.wit:22-33`、`views.mdx:129`）；图标用 IDE 图标名或包内 `.svg`（`views.mdx:34`）；双击 `Activate` → `views.open-location`（`wit/views.wit:188-193`）；刷新 `views.refresh`（`wit/views.wit:171`） | 无懒加载子节点 API，`Expand` 需插件整棵重渲染（`wit/views.wit:108`）；没有 IDE 模块对象，只有路径与符号（`wit/workspace.wit:88-105`）；视图隐藏即不渲染（`views.mdx:38`）；单次渲染 5 s（`crates/ide-plugin/src/types.rs:446`） | 模块按 `src/main/` 路径前缀分组，不还原真实模块依赖；根节点计数自算；节点超 2000 时只渲染模块+类两层，方法层按展开补；不改本体 |
| U3 服务详情：method / 完整 URL / RequestParams 键值 / RequestBody JSON（`RestServiceDetail.java:41,84`） | `detail = true` + `detail-provider`（`views.mdx:40`、`wit/details.wit:20-42`），每节点最多 8 个 tab（`wit/details.wit:47`） | pane 只有 `message`/`text`/`log`，**只读**，`text` ≤1 MiB（`wit/details.wit:23-29`，上限在 26 行）；没有输入控件，无法像旧面板那样就地改值再 Send | 详情只读预览 + 「在请求面板中打开」命令把 method/URL/参数/body 带进可编辑面板；要可编辑需改本体：`wit/details.wit:23-28` |
| U4 内置请求客户端：Send、`{var}` 替换、query 拼接、响应美化、Ctrl+Enter 格式化、双击复制（`RestServiceDetail.java:120-158`、`RequestHelper.java:49`） | 请求面板用 `Body::Form`（`wit/views.wit:43-57,73`、`views.mdx:131`）收 method/URL/参数；插件侧 `net.fetch` 发送（`wit/io.wit:56-74,96`）；响应写 content tab（`wit/views.wit:132-135,153`）或详情 `log` | 明文 HTTP 到非 loopback 必须写死 `net.http:<host>`（`wit/io.wit:48-49`、`crates/ide-plugin/src/types.rs:567-576`），`net:ask` 不覆盖明文（`crates/ide-plugin/src/wit_host.rs:1678`）；响应全量缓冲 ≤16 MiB、超限直接失败（`crates/ide-host/src/plugins/api.rs:201,2466-2471`），而注解写 8 MiB（`wit/io.wit:72`）；content tab 文本 ≤1 MiB（`src/features/plugins/views/contentTab.ts:253`）；默认 30 s 超时且被本次调用余额截断（`wit/io.wit:65-66`）；web frame 无网络（`docs/en/plugins/architecture.mdx:67`） | 默认只支持 https 与 `localhost`/`127.0.0.1`；内网明文 IP 需在 manifest 预授 `net.http:<host>`；`{var}` 替换与 query 拼接在插件内实现（对齐 `RequestHelper` 行为）；Ctrl+Enter 改为按钮/命令，双击复制改为工具栏按钮；响应 >1 MiB 截断并提示。需改本体：`types.rs:567-576`、`crates/ide-extension/src/capability.rs:155-181`（net:ask 覆盖明文）、`wit/io.wit:69-74`（8/16 MiB 统一） |
| U5 复制完整 / 相对 URL（含 `server.port` 与 context-path；`GenerateUrlAction.java:24`） | 编辑器命令（`editor.context` 菜单 `commands.mdx:157-176` + keymap 声明 `docs/en/plugins/manifest.mdx:107`）读文件文本与符号索引，拼 host:port + context-path，`ui.copy-to-clipboard`（`wit/host.wit:141-142`） | 剪贴板静默截断 1 MiB（`crates/ide-plugin/src/wit_host.rs:689`、`runtime.rs:63`）；写剪贴板只在命令/窗口事件可用（`crates/ide-host/src/plugins/api.rs:1945-1948`）；无 PSI，取不到编译期常量（`wit/fs.wit:23`） | 复制前先算长度，超 1 MiB 直接报错不复制（URL 实际远小于此）；跨类常量解析不到时保留原文 `{...}` 并提示；context-path 同时认 `server.servlet.context-path` 与旧键 `server.context-path`（`PropertiesHandler.java:161`）；不改本体 |
| U6 复制 Query Param（Key Value，跳过 `@RequestBody`，对象展开字段；`GenerateQueryParamAction.java:21`） | 同一命令族，参数来自注解文本与符号索引里的字段声明（`wit/workspace.wit:99-105`） | 无类型推断：`@RequestParam` 未写 `name`/`value` 时只能用参数名；对象只展开同项目可达字段；剪贴板 1 MiB（`wit_host.rs:689`） | 展开深度 ≤3 层，循环引用截断；解析不出的类型给 `null` 占位并在结果首行注明「含未解析类型」；不改本体 |
| U7 复制 RequestBody JSON（`GenerateQueryParamJsonAction.java:21`、`plugin.xml:136-137`） | 复用 U8 的 JSON 生成器，输入为 `@RequestBody` 的类型名 | 同 U8：无 PSI、无类型系统，`List<T>`/泛型只能按字面量猜；索引只有 class/method/field + range（`wit/workspace.wit:99-105`）；剪贴板 1 MiB | 集合给 `[]`、未知类型给 `null` 并注明；嵌套超 3 层停止展开；要精确需类型系统，等于改本体范围，不做 |
| U8 类 → JSON / JSON 压缩 / Bulk Value（`plugin.xml:166-177`） | 三条独立命令，作用于光标所在类或树节点，结果走剪贴板；不继承旧插件 static 自关联计数缺陷（`/root/job/ai-code/docs/plans/_brief.md:47`） | 无 PSI：字段只能靠文本扫描与符号索引（≤1000 条，`wit/workspace.wit:99-105`）；剪贴板 1 MiB（`wit_host.rs:689`） | 字段取本文件与索引中的同项目类；Lombok / record / 继承字段若扫不到就标「未收录」而不猜；嵌套与循环用深度 3 + 引用标记；Bulk Value 只输出 `key=value` 文本；不改本体 |
| U9 树节点右键 Copy Full Url（多选换行）/ Jump to Source；工具栏刷新（`plugin.xml:213-219`） | `[[contributes.menus]]` `menu = "viewItem.context"`（`commands.mdx:193-208`）取节点 key；跳转用 `views.open-location(path,line,col)`（`wit/views.wit:188-193`）；刷新用工具栏按钮（`views.mdx:136`） | 菜单只传**单个** item 的 key，树视图没有多选（`commands.mdx:193-198`，视图事件也没有多选类型 `wit/views.wit:104-118`）；`open-location` 行 0 基、列按 UTF-16（`wit/views.wit:188-193`） | 多选需求改为组节点上的「Copy All URLs」命令（整组换行输出）；Jump to Source 用索引里记的声明行；真要多选需改本体视图事件：`wit/views.wit:104-118` |
| U10 读 `application[-profile].properties\|yml` 的 `server.port` / context-path / profile（`PropertiesHandler.java:62,95-109,161-171`） | `project.read_file` 读配置（`wit/fs.wit:23`，≤16 MiB）后自解析 properties/yml；配置变更靠 `[[contributes.documentIndexers]]` 回调重算（`wit/indexing.wit:3-10`） | 无 profile 感知 API，`spring.profiles.active` 要自己读；旧插件只认 `server.context-path`（`PropertiesHandler.java:161`），Boot 2.3+ 用 `server.servlet.context-path`；settings 只有 `settings.get`，写不回（`wit/views.wit:196-201`） | 查找顺序 `application-{profile}.{ext}` → `application.{ext}`，properties 覆盖 yml；三个键都认；全都缺失时回退 `http://localhost:8080` 并在 URL 旁标注「默认值」；用户覆盖值走设置页由平台写回；不改本体 |
| U11 服务发现：Spring MVC + JAX-RS + Kotlin，类×方法笛卡尔积拼接归一化（`SpringResolver.java:37,52-167`、`JaxrsResolver.java:24-43`） | 插件内文本扫描器（注解名 + 括号内字符串，逐文件）+ `symbols` 索引（`wit/workspace.wit:99-105`）+ `documentIndexers` 增量回调（`wit/indexing.wit:3-10`）；结果落 `storage`（`wit/host.wit:187-190`） | 无 PSI/AST，只有 16 MiB 文本（`wit/fs.wit:23`）且 `textDocument/*` 被禁（`wit/host.wit:219-224`）；符号查询 ≤256 字节、返回 ≤1000 条；命令 5 s、task 内 60 s（`crates/ide-plugin/src/types.rs:446-447`、`commands.mdx:109-112`） | 注解值只认字面量与同类 `static final`；SpEL、`@AliasFor`、跨模块常量按「解析不出」保留原文并标记；笛卡尔积与归一化（`//` 折叠、尾斜杠）在建索引时完成；全量扫描必须开 task 分批并走 `documentIndexers`；不改本体 |
| U12 Popup 快捷菜单（Alt+Shift+R，聚合 U5–U7；`plugin.xml:113-118,129-142`） | `[[contributes.menus]]` `menu = "editor.context"` + `submenu`（`commands.mdx:153-176,185-191`）承载三条命令；`[[contributes.keymaps]]` 绑 Alt+Shift+R（`docs/en/plugins/manifest.mdx:107`） | 菜单项可带 `when`（含 `editorLangId == java`，`commands.mdx:159-171`），但**快捷键本身不能带 `when`**，Escape 是唯一例外（`commands.mdx:147-151`）；插件快捷键默认低于 IDE 与用户 keymap（`commands.mdx:131-133`） | 主入口是右键子菜单（`when` 限定 java/kotlin）；Alt+Shift+R 仅作默认绑定，可能被 keymap 或既有绑定顶掉；命令内再校验光标是否在 mapping 方法上，否则提示而非弹菜单；要快捷键按语言生效需改本体：`commands.mdx:147-151` |

汇总——需改本体 5 处：`crates/ide-host/src/plugins/endpoints.rs:99`（endpoints 2 s 硬预算，U11/U2 索引只能吃缓存）、`crates/ide-plugin/src/types.rs:567-576` + `crates/ide-extension/src/capability.rs:155-181`（net:ask 不覆盖明文，U4）、`wit/endpoints.wit:9-20`（endpoint 缺 group 字段，U2/U9 分组不下发）、`src/features/http-client/generatedRequests.ts:9,45-58`（插件调不到内建请求生成，U4）、`wit/io.wit:69-74`（响应 8/16 MiB 注释与实现不一致，U4）。

# 分册 2 · §4 Epic E1–E2：路由索引与路由浏览

> 引用基线见 `_brief.md`：未标注的路径相对 `/root/job/ai-code/openidea`，旧插件路径相对 `/root/job/ai-code/restful-toolkit-fix`；命令 id 写作 `plugin.<id>.<cmd>`。

## Epic E1 路由索引

目标：在没有 PSI/AST 的 WASM 沙箱里用自写文本解析建立可持久化、可增量的路由索引，并让 `endpoints` 在 2 s 硬预算内只读缓存发布。

### US-E1-01 单文件 Spring 注解解析

- **故事句**：作为后端开发，我希望插件只凭单个 Java 文件的文本就解析出该文件的全部 Spring 路由，以便不开项目、不依赖 PSI 也能先验证解析是否正确。
- **验收标准**：
  - Given 类上有 `@RestController` 与 `@RequestMapping("/api")`、方法上有 `@GetMapping("/users")`，When 解析该文件，Then 得到 `GET /api/users`，`location` 指向方法声明行（0 基行、UTF-16 列）。
  - Given 类上无 `@RequestMapping`、方法上 `@PostMapping("/orders")`，When 解析，Then 路由为 `/orders`（方法路径按绝对路径处理）。
  - Given `@RequestMapping(value = {"/a", "/b"}, method = {RequestMethod.PUT, RequestMethod.PATCH})`，When 解析，Then 产出 4 条路由（value × method 笛卡尔积）。
  - Given 注解值是跨类常量或无法求值的占位符，When 解析，Then 仍登记路由但标 `unresolved`、保留原文，不猜测展开，并在状态行给出计数。
- **技术要点**：宿主没有 PSI/AST 接口——入参只有文件文本（`host::read-file` ≤16 MiB，`wit/fs.wit:23-25`）、受限符号索引（≤1000 条，`wit/workspace.wit:99-105`），且 `textDocument/*` 等 LSP 命名空间被宿主拒绝转发（`wit/host.wit:220-224`）；因此解析器是自写词法扫描器：跳行/块注释与字符串字面量、按 `{}` 深度判定类体/方法体、按注解全名与简名取值，`offset` 统一换算成 0 基行 + UTF-16 列。
- **依赖**：无（首个故事，交付可独立单测的纯文本解析函数）。
- **规模**：M
- **映射**：U11

### US-E1-02 全项目路由索引与增量更新

- **故事句**：作为后端开发，我希望插件在项目级建一份路由索引并随文件改动增量更新，以便 Services 树、搜索、Endpoints 搜索在任意时刻都能秒开且不重复扫全项目。
- **验收标准**：
  - Given 首次打开含 2000 个 Java/Kotlin 文件的项目，When 执行「Rebuild Route Index」命令，Then 以 `tasks.begin` 展示进度、60 s 内完成，并把每个文件的路由按文件路径键写入 `storage`。
  - Given 索引已就绪，When 宿主调用 `endpoints`，Then 只读内存索引（必要时从 `storage` 反序列化），不读任何源码文件，整体 ≤2 s，返回条数 ≤1 万。
  - Given 某控制器文件保存，When `document-indexer` 回调该文件，Then 只重算该文件并替换其索引条目，随后 `views.refresh` + endpoints 重算；同项目其他文件不重读。
  - Given 索引尚未建好或上次构建失败，When `endpoints` 被调用，Then 返回上一版（可为空）并在状态行提示「索引构建中」，绝不返回半成品或非法路径。
- **技术要点**：`[[contributes.endpoints]] files = ["**/*.java", "**/*.kt"]`（1–64 个 glob，`crates/ide-host/src/plugins/endpoints.rs:27-28`）；`endpoints` 调用硬预算 2 s（`endpoints.rs:99`），失败保留上一版（`endpoints.rs:111-118`），超 1 万条或 4 MiB 整批作废（`endpoints.rs:181-193`），故路径合法性（前导 `/`、无空白）必须在索引层先过滤；`endpoint` record 只有 `method?/path/framework?/handler?/location`，**无 module/group 字段**（`wit/endpoints.wit:9-20`），模块分组只能留在插件自己的索引里；增量来源是 `[[contributes.documentIndexers]] extensions = ["java", "kt"]`（≤32 个扩展，`crates/ide-host/src/plugins/navigation.rs:581-590`）+ `tasks.begin`（`docs/en/plugins/lifecycle.mdx:74`）；`storage` 64 MiB 跨重启（`wit/host.wit:187-190`），闲置 300 s 回收后从 storage 重建（`docs/en/plugins/lifecycle.mdx:24`）。
- **依赖**：US-E1-01。
- **规模**：L
- **映射**：U11

### US-E1-03 类级与方法级路径拼接归一化

- **故事句**：作为后端开发，我希望类级路径与方法级路径按 Spring 语义拼成唯一稳定的 URL，以便 `@RequestMapping("/api/")` + `@GetMapping("/users")` 不会产出 `//users` 或尾斜杠。
- **验收标准**：
  - Given 类路径 `/api/` 与方法路径 `/users`，When 拼接，Then 得到 `/api/users`（边界只留一个 `/`）。
  - Given 类路径 `/api//v1`，When 拼接，Then 中间的重复 `/` 折叠为 `/api/v1`；`{id:[0-9]+}` 内的字符原样保留。
  - Given 方法路径为空字符串或 `/`，Then 结果就是类路径本身（去掉尾斜杠，根路径保留 `/`）。
  - Given 类上有两个 mapping、方法上有两个路径，When 拼接，Then 输出 4 条去重后的 URL，且每条都以 `/` 开头。
- **技术要点**：纯函数 `join(class_paths, method_paths) -> Vec<String>`，规则固化在单测里；旧实现只补边界斜杠（`restful-toolkit-fix/src/main/java/com/zhaow/restful/common/resolver/BaseServiceResolver.java:74-84`、`.../method/RequestPath.java:29-41`），空方法路径会留尾斜杠，本故事明确**不复刻**该缺陷。
- **依赖**：US-E1-01。
- **规模**：S
- **映射**：U11

### US-E1-04 JAX-RS 与 Kotlin 支持

- **故事句**：作为维护混合技术栈的后端开发，我希望同一套索引也覆盖 JAX-RS 注解与 Kotlin 源码，以便非 Spring 服务同样能在树和搜索里被找到。
- **验收标准**：
  - Given 类 `@Path("/api")`、方法 `@Path("/users/{id}")` + `@GET`，When 解析，Then 得到 `GET /api/users/{id}`，`framework` 标为 JAX-RS。
  - Given 方法同时标 `@POST` 与 `@PUT`，Then 每个 HTTP 方法各产出一条同路径路由。
  - Given Kotlin `@RequestMapping(value = ["/k"], method = [RequestMethod.POST])`（无分号、数组用 `[]`），When 解析，Then 与 Java 版本结果一致。
  - Given Kotlin 注解带 use-site target（`@get:GetMapping`）或全限定名（`jakarta.ws.rs.GET`），Then 剥离前缀后仍能识别；同一类同时含两套框架注解时各出一组、不混合。
- **技术要点**：在 E1-01 的扫描器上并列两张注解表（Spring 与 JAX-RS）；注解名比较用「去包名后的简名」；Kotlin 需额外处理 `value = [..]`、`method = [..]`、`@get:`/`@field:` 前缀；旧实现同样是双 resolver + Kotlin 分支（`restful-toolkit-fix/src/main/java/com/zhaow/restful/common/resolver/SpringResolver.java:133-260`、`.../resolver/JaxrsResolver.java`、`.../common/KtFunctionHelper.java`）。
- **依赖**：US-E1-01、US-E1-03。
- **规模**：M
- **映射**：U11

### US-E1-05 配置文件生成 base URL（server.port/context-path/profile）

- **故事句**：作为联调中的后端开发，我希望插件读取 `application[-profile].properties|yml` 推导出 base URL，以便复制的地址自动带上正确端口与 context-path，而不是永远 8080。
- **验收标准**：
  - Given `application.yml` 有 `server.port: 8081`、`server.servlet.context-path: /gateway`，When 生成 base URL，Then 为 `http://localhost:8081/gateway`。
  - Given `application-dev.yml` 覆盖 `server.port: 9090` 且 `spring.profiles.active=dev`，Then 取 9090；把 active profile 改成 `prod` 后同一路由的 URL 随之变化。
  - Given 无任何配置文件，Then 默认 `http://localhost:8080`（无 context-path）；Given `server.port=${PORT:8080}`，Then 取冒号后的默认值 `8080`，不输出占位符原文。
  - Given 只写了旧键 `server.context-path`，Then 仍识别；Given 配置文件被修改，Then base URL 缓存按 `project.stat` 的 `modified` 失效并重算。
- **技术要点**：`project.find-files`（glob 如 `**/application*.yml`、`**/application*.properties`，`wit/workspace.wit:90-95`）+ `host::read-file` 读取；优先级沿用旧实现 `application > bootstrap`、`properties > yml`（`.../method/action/PropertiesHandler.java:40-49`），profile 取 `spring.profiles.active` 后叠加 `application-<profile>.*`（同文件 `:62-109`），context-path 兼容 `server.servlet.context-path` 与旧 `server.context-path`（同文件 `:161`），占位符取默认值（同文件 `:178-180`）；缓存键含文件路径 + `modified`（`wit/workspace.wit:24-31`）；设置只读、不写回（`wit/views.wit:198-201`）。
- **依赖**：无（与 US-E1-02 共用 storage 命名空间，先于 E3 的复制命令落地）。
- **规模**：S
- **映射**：U10

## Epic E2 路由浏览

目标：把索引变成可浏览、可跳转、可搜索的界面；树/详情受 `views`/`details` 契约限制，跳转受 0 基行 + UTF-16 列约束。

### US-E2-01 路由树视图（模块→控制器→方法）

- **故事句**：作为代码走读者，我希望在 Services 位置按「模块→控制器→方法」浏览全部路由并看到根节点计数，以便一眼判断哪个模块接口最多、入口在哪个类。
- **验收标准**：
  - Given 索引为 3 模块 / 20 控制器 / 87 方法，When 打开视图，Then 根节点显示 `Found 87 services`，其下为模块节点，模块下为 `Class#method` 节点，方法节点有图标则用图标，宿主无对应图标时降级为 `[GET]` 文本前缀（当前宿主图标集无 HTTP 动词图标，实际一律走文本前缀）。
  - Given 用户展开某模块，When `expand(key)` 事件到达，Then 重渲染后子节点可见、其他节点原有展开状态保留，单次渲染 ≤5 s。
  - Given 索引有 1 万条路由，When 渲染，Then 只渲染配置上限内的节点（默认 5000）并在状态行提示「已截断」，不静默丢弃。
  - Given 视图所在工具窗被隐藏，When 文件变化只更新索引，Then 不触发 `render`、不重算树；重新显示时从 storage 索引重建当前内容。
- **技术要点**：`[[contributes.views]] id = "routes" location = "services" detail = true`；树是扁平 `list<tree-item>`，`parent` 只能指向更早的项（`wit/views.wit:22-33`、`wit/views.wit:68-70`）；`expand` 只是事件，`handle` 无返回值（`wit/views.wit:108,211`），**没有懒加载子节点 API**，唯一更新方式是 `views.refresh` 后整棵重渲染（`wit/views.wit:171`）；模块节点由文件路径的 `/src/main/` 前缀推导（endpoints record 无 group 字段，见 US-E1-02）；`when` 为假或被隐藏时不渲染不激活（`docs/en/plugins/guides/views.mdx:38`）；渲染默认预算 5 s（`crates/ide-plugin/src/types.rs:448`）。
- **依赖**：US-E1-02、US-E1-03、US-E1-05。
- **规模**：L
- **映射**：U2

### US-E2-02 路由详情面板

- **故事句**：作为联调人员，我希望选中路由后看到 method、完整 URL、RequestParams 键值骨架与 RequestBody 字段骨架，以便不打开源码就判断参数契约。
- **验收标准**：
  - Given 选中 `GET /api/users/{id}`，Then 详情区出现 Method / Full URL / Params / Body 四个只读 tab，Params 预填 `id:`，Full URL 含 E1-05 的 base URL。
  - Given 方法无 `@RequestBody`，Then Body tab 显示 message「无请求体」而不是空白文本。
  - Given 参数或字段骨架超过 1 MiB，Then 截断并附「已截断」提示，调用不报错。
  - Given 视图尚未渲染过（插件刚被回收后重建），When 选中节点，Then 详情仍能从 storage 索引重建，不依赖上次内存。
- **技术要点**：`[[contributes.views]] detail = true` + `register_plugin!(T, details)`（SDK `sdk/rust/openidea-plugin/src/details.rs:69-72`）；`detail(view, key)` 每节点最多 8 个 tab（`wit/details.wit:47-49`），`Pane::Text` 只读且 ≤1 MiB（`wit/details.wit:26-27`），无输入控件——编辑能力全部留给 E4 的请求面板；参数键值与字段名直接来自 E1-01 已存的注解文本，JSON 预填本期只到「字段名 + 类型占位」。
- **依赖**：US-E2-01。
- **规模**：M
- **映射**：U3

### US-E2-03 双击跳转源码

- **故事句**：作为读代码的人，我希望双击路由节点直接跳到声明该路由的源码位置，以便从 URL 一步回到 Controller 方法。
- **验收标准**：
  - Given 双击方法节点，When `activate(key)` 到达，Then 调用 `views.open_location` 打开对应文件，光标落在方法注解行首，行列为 0 基 + UTF-16。
  - Given 方法声明前有中文或 emoji，Then 列号按 UTF-16 码元计算，落点与声明字符一致（用 MockHost 编辑器断言）。
  - Given 双击模块节点或控制器节点，Then 打开控制器类声明行（模块节点则展开而不跳转），不报错。
  - Given 索引中的文件已改名导致位置失效，Then 提示「源码位置已失效，请刷新索引」，不打开错误位置；右键菜单 Jump to Source 与双击走同一命令。
- **技术要点**：`ViewEvent::Activate(key)`（`wit/views.wit:105-106`，SDK `sdk/rust/openidea-plugin/src/views.rs:267-269`）→ `views.open_location(path, line, column)`，0 基行、UTF-16 列（`wit/views.wit:189-193`，SDK `sdk/rust/openidea-plugin/src/host.rs:988`）；节点 key 只编码「文件路径 + 行 + 列 + 控制器/方法名」，不放整段文本；同一命令挂 `viewItem.context` 菜单（`docs/en/plugins/guides/commands.mdx:168,193-205`），`when` 用 `view == routes` + `viewItemKind` 区分节点类型。
- **依赖**：US-E2-01、US-E1-01（location 精度）。
- **规模**：S
- **映射**：U2、U9

### US-E2-04 按 URL 搜索路由

- **故事句**：作为拿到一条 URL 的联调人员，我希望在 Search Everywhere 里输入 URL 片段（可用 `*` 通配）就直接定位到路由声明，以便不必先找到 Controller 再逐层展开。
- **验收标准**：
  - Given 输入 `/api/v1/orders/*`，When 查询本插件分类，Then 命中 `GET /api/v1/orders/{id}` 等全部同前缀路由，回车跳到声明行。
  - Given 粘贴完整 URL `http://10.0.3.7:8080/gateway/api/v1/orders/42`，Then 自动去掉 scheme/host/port 与本项目 context-path 后按路径后缀命中。
  - Given 查询为空，Then 返回按模块排序的前 20 条作为初始建议；整个搜索回调不读剪贴板、不弹窗。
  - Given 查询超过 1024 UTF-8 字节或 limit 超出 1–200，Then 宿主按契约拒绝/截断，插件不 panic。
- **技术要点**：`[[contributes.searchCategories]] id = "routes"`（API 1.13，`docs/en/plugins/manifest.mdx:114`）+ `register_plugin!(T, search)`；`search(category, query, limit)` 是后台调用，禁止提示与隐式命令，query ≤1024 字节、limit 1–200、结果按自身排序返回（`wit/search.wit:22-26`）；结果用 `search-target::location` 直接给跳转位置（`wit/search.wit:13-17`），不需要剪贴板；`*` 语义沿用旧 `AntPathMatcher`（`restful-toolkit-fix/src/main/java/com/zhaow/restful/common/spring/AntPathMatcher.java`）。内建 Endpoints 段已覆盖字面路径（`src/features/search/everywhere/routes.ts`、`src/features/search/everywhere/controller.ts:393-400`），本分类只补通配与完整 URL 归一化；旧 Ctrl+\ 的剪贴板预填降级为命令 `plugin.<id>.searchRoute`（用户主动运行才允许 `read-clipboard`，`wit/host.wit:150-157`）。
- **依赖**：US-E1-02、US-E1-03、US-E1-05。
- **规模**：M
- **映射**：U1

# 第三分册 · Epic E3 生成与复制 / Epic E4 请求发送与响应

- 基线：`/root/job/ai-code/openidea` @ `097221c1b`；旧插件记作 **RT**（`/root/job/ai-code/restful-toolkit-fix` @ `3fd3aaa`）。未注明者的行号相对 openidea。
- 本册只含 E3（4 个故事）与 E4（3 个故事）；E1 的解析、E2 的树、E5 的类→JSON 不在此重复。
- **共享事实（后文只引行号）**
  - 剪贴板 1 MiB：`host::copy_to_clipboard` → `ui.copy-to-clipboard`（wit/host.wit:141-142、sdk/rust/openidea-plugin/src/host.rs:207），宿主侧 `clip(text, MAX_TEXT)` **静默截断**，`MAX_TEXT = 1 MiB`（crates/ide-plugin/src/runtime.rs:63、crates/ide-plugin/src/wit_host.rs:689）；`MockHost` 不截断（sdk/rust/openidea-plugin/src/mock.rs:1145，取值用 `clipboard()`，mock.rs:509），所以「判长 + 提示」必须写在插件里才能被单测覆盖。
  - `when`：`editorLangId == java` 可写在命令与菜单上（docs/zh/plugins/guides/commands.mdx:40,143），但 `[[contributes.keymaps]]` 只有 `command/key/mac`（同文件:104-117），快捷键带不出 `when`——「只在 Java 编辑器生效」一律落在命令自身。
  - 右键：`viewItem.context` 必须带 `view`（plugins/README.md:152,164-177；crates/ide-host/src/plugins/ui_points.rs:92,181-190），命令参数就是被点节点的 key（src/features/plugins/uiPoints.tsx:163-175）。
  - 多选：宿主视图单选（src/features/plugins/views/PluginView.tsx:364 `multiSelect={false}`），右键只给一个 key，旧插件 JTree 的多选在宿主不存在，等价实现见 US-E3-04。

---

## Epic E3 生成与复制

一条管线：解析（E1）→ 拼装 → 判长 → 写剪贴板。四个故事共用 `[[contributes.commands]]` 的 `when = "editorLangId == java"` 与 `[[contributes.menus]] menu = "editor.context"`（Kotlin 由 `.kt` 兄弟命令以 `editorLangId == kotlin` 承接）。

### US-E3-01 复制完整 URL

- **故事句**：作为后端开发，我希望把光标所在方法变成一条可直接粘贴的完整 URL，以便立刻在浏览器或 Postman 里验证接口地址。
- **验收标准**：
  - Given `application.yml` 有 `server.port: 8080` 与 `servlet.context-path: /api`，光标在 `@GetMapping("/{id}")` 方法内，When 执行命令，Then `MockHost::clipboard()` 等于 `http://localhost:8080/api/v1/users/{id}`（`/` 归一化由 US-E1-01 保证）。
  - Given 光标在类声明行或注解之外，When 执行命令，Then 剪贴板保持旧值并 `ui.notify` 提示「光标不在映射方法内」，不写入半成品。
  - Given 拼接结果超过 1 MiB（超长 `@RequestMapping` 值的合成用例），When 执行，Then 只写前 1 MiB 且 notify 说明已截断——宿主会静默截断，插件必须先判长（runtime.rs:63、mock.rs:1145）。
  - Given 文件是 Kotlin，When 触发同名命令，Then 不可用（`editorLangId == java` 不成立），由 `copy-full-url.kt` 承接。
- **技术要点**：`[[contributes.commands]] id="copy-full-url" when="editorLangId == java"` + `[[contributes.menus]] menu="editor.context"`（同 `when`）；`inv.selections()` 定位光标行（wit/workspace.wit:159），`text-editor.document()` 取全文（wit/workspace.wit:156）；`run` 的 `arg = "relative"` 时输出相对 URL（wit/exports.wit:22）；能力 `editor.read` + `clipboard`。
- **依赖**：US-E1-01、US-E1-03。
- **规模**：S
- **映射**：U5

### US-E3-02 复制 Query Param（Key Value）

- **故事句**：作为联调者，我希望一键拿到该方法所有 query 参数的 `k=v` 骨架，以便直接拼进 URL 或丢给 curl。
- **验收标准**：
  - Given 方法签名 `(String name, int age, @RequestBody User body)`，When 执行，Then 剪贴板为 `name=&age=0`：`@RequestBody` 参数被跳过、基础类型给默认值、以 `k=v&k=v` 拼接（复刻 RT `PsiMethodHelper.java:67-80,84-126`）。
  - Given 参数是自定义对象 `PageQuery{int pageNo; int pageSize;}`，When 执行，Then 展开为 `pageNo=0&pageSize=0`，`serialVersionUID` 等静态常量字段不出现（RT 同文件:108-122）。
  - Given 参数写作 `@RequestParam("q") String keyword` 或带 `defaultValue`，Then 键取注解名、值取 `defaultValue`，不泄漏 Java 变量名。
  - Given 方法没有非 body 参数，Then 写入空串并 notify 说明无可复制参数（不写孤立的 `?`）。
- **技术要点**：无 PSI，按 E1 的参数切片做文本解析（注解名/类型/默认值）；对象字段用 `project.symbols(query, limit)` 定位类（wit/workspace.wit:105）后读字段，字段遍历复用 US-E5-01；命令 `copy-query-param`，菜单同 E3-01；判长逻辑同共享事实。
- **依赖**：US-E1-01、US-E5-01。
- **规模**：M
- **映射**：U6

### US-E3-03 复制 RequestBody JSON

- **故事句**：作为后端开发，我希望复制出 `@RequestBody` 类型的 JSON 骨架，以便改几个字段值就能发请求。
- **验收标准**（占位值逐项沿用 US-E5，**口径以 US-E5 为唯一来源**，本故事不另立数值）：
  - Given `@PostMapping` 方法有 `@RequestBody UserCreateReq req`，When 执行，Then 剪贴板为 `UserCreateReq` 的 2 空格缩进 JSON：字段顺序与声明一致，嵌套对象递归展开到 `maxDepth`（默认 4），超出深度写 `{}`。
  - Given 方法有多个参数但只有一个带 `@RequestBody`，Then 只取那一个；Given 一个都没有，Then notify「该接口没有 @RequestBody」，剪贴板不变。
  - Given 字段类型是 `String` / `boolean` / `int` / `List<User>` / `Map<String,Object>` / 未知类型，Then 值分别为 `"demoData"` / `true` / `1` / `[]` / `{}` / `null`（与 US-E5 占位值表逐项相同），不报错。
  - Given 生成结果超过 1 MiB（深层嵌套合成用例），Then 截断并提示。
- **技术要点**：命令 `copy-request-body`，`when = "editorLangId == java"`，菜单 `editor.context`；类型名 → `project.symbols` 找类（wit/workspace.wit:105）→ 复用 US-E5-01 的类→JSON；序列化在插件内完成，先判 1 MiB 再调 `host::copy_to_clipboard`。
- **依赖**：US-E1-01、US-E5-01。
- **规模**：S
- **映射**：U7

### US-E3-04 聚合快捷菜单与树节点右键

- **故事句**：作为频繁切换接口的人，我希望一个入口（快捷键或树上右键）就能选「拷 URL / 拷 Query / 拷 Body」，需要多条时一次拷完，以便不必记三个命令名。
- **验收标准**：
  - Given Java 文件聚焦，When 按 `Alt+Shift+R`（`[[contributes.keymaps]]`，本身无 `when`），Then `ui.pick` 列出三项（wit/host.wit:164），选中项转调 US-E3-01/02/03 的实现。
  - Given 焦点在非 Java 编辑器，When 按同一快捷键，Then 命令因自身 `when = "editorLangId == java"` 不执行（快捷键无法带 `when`，只能靠命令兜底）。
  - Given 在树上右键 `GET /users` 节点并选「Copy Full Url」，Then 剪贴板是该节点完整 URL（`viewItem.context` 的 arg 即节点 key，src/features/plugins/uiPoints.tsx:163-175）。
  - Given 右键控制器/根节点（代表多条端点），When 选「Copy Full Url」，Then 其子树全部 URL 以 `\n\n` 拼接（沿用 RT `CopyFullUrlAction.java:25-32` 的分隔）；需要挑不相邻的多条时用 `restful.copy-many` + `ui.pick-many`（wit/host.wit:169）选择后同样 `\n\n` 拼接。拼接结果超 1 MiB 则截断并提示。
- **技术要点**：`[[contributes.menus]] menu="viewItem.context"` + `view="restful"`（crates/ide-host/src/plugins/ui_points.rs:92,181-190；plugins/README.md:152,164-177）；聚合命令 `restful.pick` 用 `ui.pick`，多选用 `ui.pick-many`；对应上面第 4 条，这就是「多选节点换行拼接」在单选宿主（`PluginView.tsx:364`）下的等价落地。
- **依赖**：US-E3-01、US-E3-02、US-E3-03、US-E2-01。
- **规模**：M
- **映射**：U9、U12

---

## Epic E4 请求发送与响应

权限是硬门槛，预算靠 task 续命，响应全量缓冲。插件无法复用内建 HTTP Client（src/features/http-client/generatedRequests.ts:9,45-58），请求面板与发送链路自建。

### US-E4-01 发送请求

- **故事句**：作为后端开发，我希望在 IDE 里对当前端点直接发一次请求，以便不离开编辑器就看到返回。
- **验收标准**：
  - Given 面板填好 `GET http://localhost:8080/api/users/1`，When 点 Send，Then `net.fetch` 的状态与 body 渲染到响应区；4xx/5xx 也按正常响应展示（宿主只把网络层错误当 `failed`，crates/ide-host/src/plugins/api.rs:2455-2478）。
  - Given 目标 host 是 HTTPS 或 loopback 且 manifest 只声明 `net:ask`，When 首次发送，Then 弹授权框，允许后记入台账、再发不问；无窗口的调用路径直接 `denied`（plugins/README.md:1763-1776）。
  - Given 目标是明文 `http://192.168.1.10:8080` 而 manifest 只有 `net:ask`，When 发送，Then `denied`——`net:ask` 只覆盖 HTTPS 与 loopback，非 loopback 明文必须写死 `net.http:192.168.1.10`（**需改本体: crates/ide-plugin/src/types.rs:567-576**；crates/ide-extension/src/capability.rs:155-181 只做 pattern 覆盖，补不上明文场景）。
  - Given 响应体 20 MiB，When 发送，Then 得到 `failed("response larger than 16 MiB")`，面板显示可读错误而非空 body（`MAX_RESPONSE = 16 << 20`，api.rs:202,2469-2470）。
  - Given 一次请求要 40 s，When 发送，Then 不超时：`host::begin_task` 把本次调用续到 60 s（wit/events.wit:53-68、sdk/rust/openidea-plugin/src/host.rs:1625），期间按钮禁用并显示进度；未开 task 时默认 30 s 且被剩余预算再截断（crates/ide-plugin/src/wit_host.rs:1701-1707）。
- **技术要点**：`[[contributes.views]] id="request"`，`body = form` + toolbar `send`，事件经 `handle` 回传（wit/views.wit:83,102-120）；`[capabilities] required = ["net:ask"]` 且 `engines.openidea = "^1.5"`（plugins/README.md:1763-1773）；请求 body ≤8 MiB（wit_host.rs:245,1698）；**无 cookie jar**：每次 `fetch` 现建 client，`build_with_redirect` 不带 cookie store（api.rs:2436-2454），需要登录态的接口由用户自己在 headers 贴 `Cookie`；重定向 ≤10 且逐跳校验（api.rs:2436-2454）。
- **依赖**：US-E1-03、US-E2-03；明文非 loopback 场景需改本体: crates/ide-plugin/src/types.rs:567-576。
- **规模**：L
- **映射**：U4

### US-E4-02 参数替换与 query 拼接

- **故事句**：作为联调者，我希望请求里的 `{var}` 与 query 按变量表自动拼好，以便改一次变量就重发，不用手改 URL。
- **验收标准**：
  - Given URL `…/api/users/{id}` 且变量表 `id=42`，When 发送，Then 实际请求 `/api/users/42`；Given 变量缺失，Then 保留 `{id}` 原样、notify 缺哪几个变量并放弃发送。
  - Given base 已含 `?debug=1` 且参数表 `name=alice, age=0`，Then 得 `?debug=1&name=alice&age=0`（已有 `?` 用 `&`，否则用 `?`，复刻 RT `PsiMethodHelper.java:268-275,298-310`）。
  - Given 参数值含空格/中文/`&`，Then 值做 percent-encoding，而 `{id}` 这类路径占位符在替换前不编码。
  - Given 替换后的 host 与声明不一致，When 发送，Then 按新 URL 重算所需能力并走授权或拒绝（`net_capability`，crates/ide-plugin/src/types.rs:567-576；`Capability::covers`，crates/ide-extension/src/capability.rs:155-166）。
- **技术要点**：变量表存 `storage`（wit/host.wit:190）；替换与拼接全在插件内（宿主无此 API）；表单字段变更走 `view-event field`（wit/views.wit:111）；拼接结果交 US-E4-01 的发送链路。
- **依赖**：US-E4-01、US-E3-02、US-E3-03。
- **规模**：M
- **映射**：U4

### US-E4-03 响应展示与错误处理

- **故事句**：作为读返回的人，我希望响应按状态、头、美化后的 body 分块展示并能一键复制，出错时看到人话原因。
- **验收标准**：
  - Given 返回紧凑 `{"a":1}`，When 渲染，Then 面板显示状态码、耗时、`Content-Type` 与缩进后的 JSON；点工具栏 Format 在美化/压缩之间切换（格式化不绑 Ctrl+Enter：快捷键带不出 `when`，会抢编辑器键位，降级为按钮）。
  - Given 响应体 3 MiB，When 渲染，Then 只渲染前 1 MiB 并标注已截断（`Pane::Text` 上限 1 MiB，wit/details.wit:26），复制仍受 1 MiB 剪贴板上限（runtime.rs:63）。
  - Given `denied`，Then 提示「该 host 未授权：改用 HTTPS 或声明 net.http:<host>」；Given `cancelled`，Then 静默收起；Given 超时/超 16 MiB，Then 各自独立文案（错误枚举 wit/types.wit:8-18）。
  - Given 点「Copy Body」或双击 body 区，Then 走 `host::copy_to_clipboard`，与 E3 共用判长与截断提示。
- **技术要点**：响应面板二选一：`details` 的 `Pane::Text`（只读可选中，wit/details.wit:26）或视图 `body = form` + `table`；JSON 美化用插件内解析器（宿主无格式化 API）；Format/Copy 走 toolbar `button`（wit/views.wit:104）。
- **依赖**：US-E4-01。
- **规模**：M
- **映射**：U4

## Epic E5 数据模型转 JSON

> US-E5-01 类 → 示例 JSON；US-E5-02 压缩 JSON；US-E5-03 Bulk Value（field:value 多行）

**E5 共同基线（三个故事共用，只定义一次）**
- 可用的只有文本 + 索引：当前文档 `text-editor.document`（`wit/workspace.wit:156`，单文件 ≤16 MiB `wit/fs.wit:23`）与 `project.symbols(query, limit)`（query ≤256 字节、`limit` 上限 1000，`wit/workspace.wit:99-105`）。`symbol-info` 只有 name/kind/container/location，**没有类型字段**（`wit/workspace.wit:34-40`）→ 类型只能回读该字段声明所在文件的文本再解析；索引只用于定位类与继承链，不当作类型来源。
- 解析边界（解析不出就降级，不猜）：类型归一顺序为 同文件嵌套类 → 同包 → 本文件 import → `java.lang`/`java.util`；全部失败即「未解析」，JSON 值写 `null`、Bulk Value 写 `demoData`，并在通知里列出 `类.字段` 清单；注解值只认字面量与同文件常量，跨类常量、SpEL、方法调用一律按未解析处理。
- 占位值表（唯一来源）：`byte/short/int/long/Integer/…`→`1`；`double/float/BigDecimal`→`1`；`boolean`→`true`；`char/Character`→`"Z"`；`String`→`"demoData"`；`Date/LocalDateTime/LocalDate`→`"2024-01-01 00:00:00"`（固定串，不读时钟，保证可复现）；枚举→首个常量名，解析不到→`"demoData"`；`List/Set/数组`→`[]`，`Map`→`{}`；未解析→见上一条。

### US-E5-01 类 → 示例 JSON
- **故事句**：作为后端开发，我希望在类名或类体内执行一个命令就把该类字段复制成带占位值的 JSON，以便立刻拿去当请求体或单测骨架。
- **验收标准**：
  - Given 光标在 `UserDto` 类体内、该类有 8 个基础类型字段 + 一个 `AddressDto` 嵌套 + `List<TagDTO>`，When 执行命令，Then 剪贴板得到缩进 JSON，字段顺序与源码声明一致（旧实现用 `HashMap` 收集，顺序不稳定，`restful-toolkit-fix/src/main/java/com/zhaow/restful/common/PsiClassHelper.java:73-79`）。
  - Given 某字段类型是跨模块类且 `symbols` 查不到，When 转换，Then 该字段为 `null`，通知列出未解析字段，命令仍成功。
  - Given 自关联 `Node { Node parent; }`，When 连续执行两次，Then `parent` 为 `{}` 且两次输出逐字节相同（旧实现 static 计数从不清零，`PsiClassHelper.java:23,151-155`、`KtClassHelper.java:22`）。
  - Given 光标不在 Java/Kotlin 类声明内，When 执行命令，Then 提示「把光标放在类名或类体内」，不写剪贴板。
  - Given MockHost 的符号索引返回空（模拟索引未就绪），When 转换当前文件里的类，Then 仍能用纯文本解析产出 JSON。
- **技术要点**：命令 `plugin.openidea-restful-toolkit.class-to-json` 挂 `editor.context`，`when = "editorLangId == java || editorLangId == kotlin"`（`docs/en/plugins/guides/commands.mdx:33,157-166`）；光标 offset 取 `selections()`（`wit/workspace.wit:159`），向上做花括号配对找最近的 `class|record|interface|enum`；继承与嵌套用 `symbols(FQCN, 1000)` 定位后再 `read_file` 取文本；每次调用新建「路径 visited 集合 + maxDepth（默认 4）」的上下文，只有当前路径重复才截断，兄弟重复不截断；复制走 `copy-to-clipboard`（`wit/host.wit:142`，需 `clipboard`），结果 >900 KiB 直接提示而不交给 1 MiB 静默截断（简报硬约束 5，`docs/plans/_brief.md:17`）。
- **依赖**：无（文案键与权限清单由 US-E6-02、US-E6-03 统一收口，键名固定，不阻塞本故事）；若希望索引直接给类型，需改本体:wit/workspace.wit:34-40。
- **规模**：M
- **映射**：U8

### US-E5-02 压缩 JSON
- **故事句**：作为联调的人，我希望同一份示例 JSON 有单行版本可直接粘进 curl 或命令行，以便不必手工删换行。
- **验收标准**：
  - Given 与 US-E5-01 同一个类，When 执行压缩命令，Then 输出解析后与缩进版语义相等（MockHost 里按 `serde_json::Value` 比较）。
  - Given 字段值含引号、反斜杠、换行与中文，When 压缩，Then 输出是合法单行 JSON，中文按 UTF-8 原样（不转 `\uXXXX`）。
  - Given 类没有任何字段，Then 输出 `{}`，不是空字符串。
  - Given 输出超过 900 KiB，Then 提示「内容过大」并把内容写入 `fs.plugin_dir` 文件，不调用剪贴板。
- **技术要点**：与 US-E5-01 共用 `model::build(ClassRef, Options { pretty })`，字段收集与序列化只有一个内核，压缩只是序列化开关（旧实现是两条独立路径，`ConvertClassToJSONAction.java:24` 与 `ConvertClassToJSONCompressedAction.java:21`）；序列化用插入序 map，不依赖哈希序；JSON 写出器自己实现，保证占位值表与转义规则单一来源。
- **依赖**：US-E5-01。
- **规模**：S
- **映射**：U8

### US-E5-03 Bulk Value（field:value 多行）
- **故事句**：作为写测试数据的人，我希望一次拿到「字段:占位值」的文本清单，以便直接贴进 properties 或测试用例。
- **验收标准**：
  - Given `Child extends Base`，Base 有 `id`、Child 有 `name`，When 执行 Bulk Value，Then 输出两行 `id:1` 与 `name:demoData`，父类在前，结尾无空行（旧实现每行补 `\r\n`，末尾多一个空行，`ConvertClassToBulkValueAction.java:38`）。
  - Given 字段类型为 `List<String>` 或自定义对象，Then 值为 `demoData`，不写 `Type:null`（旧实现拼出 `List<String>:null`，同文件 :38）。
  - Given 文本 `A extends B`、`B extends A`（或超过深度上限），When 解析，Then 停止向上并通知，不挂起、不 OOM。
  - Given 设置 `bulkValue.lineEnding = "lf"`，Then 行尾为 LF；键不存在时用内置默认 CRLF。
  - Given 类里有 `serialVersionUID`，Then 默认跳过；`bulkValue.skipSerialVersionUid = false` 时保留。
- **技术要点**：继承链靠文本 `extends` + `symbols` 定位（`wit/workspace.wit:105`），深度上限取 `json.maxDepth`（默认 4，超出即止）；行尾与跳过项读 `settings.get`（`wit/views.wit:200`），缺失或类型不符回退内置默认；值走 E5 共同占位值表，不复制旧的 `getJavaBaseTypeDefaultValue` 分支（`PsiClassHelper.java:88-108`）；多类同名字段保留子类声明并保持父类在前，避免覆盖顺序随哈希变化。
- **依赖**：US-E5-01（解析内核）；设置键 `bulkValue.*` 未配置时用内置默认（键定义见 US-E6-01）。
- **规模**：S
- **映射**：U8

## Epic E6 配置与本地化

> US-E6-01 设置页；US-E6-02 本地化；US-E6-03 权限最小化与可读错误提示

- 设置只有读取 API：`settings.get(key) -> option<string>`（值是 JSON 文本，`wit/views.wit:198-201`），没有 `settings.set`；写回一律由 IDE 生成的设置页负责（`docs/en/plugins/lifecycle.mdx:86-107`）。
- 目录只翻译 manifest 声明文本与视图文本；`format!` 与通知拼出来的串不会自动翻译（`docs/en/plugins/manifest.mdx:142`）。

### US-E6-01 设置页
- **故事句**：作为团队里的使用者，我希望在 IDE 设置里调整深度上限、行尾和跳过项，以便同一插件适配不同项目约定且重启后仍生效。
- **验收标准**：
  - Given 安装并启用插件，When 打开 Settings › Plugins，Then 有「Restful Toolkit」页，字段带 title/default、可搜索，project 层可覆盖 global。
  - Given 改动 `json.maxDepth` 并 Apply，When 再次执行 US-E5-01，Then 新上限生效；无需重载插件（订阅 `settings-changed` 需 `events`，`docs/en/plugins/lifecycle.mdx:48`）。
  - Given 用户什么都没设，Then `settings.get` 返回 none，代码用内置默认（maxDepth 4、lineEnding crlf、skipSerialVersionUid true），行为与显式设成默认一致。
  - Given 存储里的值与 schema 类型不符，Then 读取回退默认值，不报错（`lifecycle.mdx:105`）。
  - Given 设置页上的 action「测试解析」，Then 先应用待保存值再运行命令，命令失败只在按钮旁报错（`plugins/README.md:2585-2587`）。
- **技术要点**：`[contributes.settings]` 单页 `schema` 或多页 `[[contributes.settings.pages]]`（用 pages 需 `engines.openidea = "^1.3"`，`plugins/README.md:2524-2529`；上限 32 页/200 键/每页 32 action，`plugins/README.md:2579-2580`）；读取统一封装 `config::max_depth()/line_ending()/skip_serial_uid()`，键名 `json.maxDepth`、`bulkValue.lineEnding`、`bulkValue.skipSerialVersionUid`；插件内不申请写回，因为 API 不存在。
- **依赖**：无（被 US-E5-01、US-E5-03 读取）；若要在插件内写设置，需改本体:wit/views.wit:198-201。
- **规模**：S
- **映射**：U8

### US-E6-02 本地化
- **故事句**：作为中文用户，我希望菜单、视图标题、树节点和提示跟随 IDE 语言，以便不用在中英之间猜。
- **验收标准**：
  - Given IDE 语言为 zh-CN 且 `locales/zh-CN.json` 有条目，Then 命令标题、菜单、视图标题、树节点标签显示中文；缺条目回退英文原文（`docs/en/plugins/manifest.mdx:136-142`）。
  - Given 运行期拼出的「找到 12 个服务」，Then 它不是靠目录翻译，而是 `locale()` 选表后从插件自带 `locales` 读模板替换；切换语言后下一次渲染即变（`wit/host.wit:184`）。
  - Given 用户语言为 fr 且没有 `fr.json`，Then 全部回退英文，无空标签、无 panic。
  - Given `openidea-plugin check`，Then 两个目录文件都通过（必须是 string→string 的 JSON 对象）。
- **技术要点**：`locales/en-US.json`、`locales/zh-CN.json`；manifest 与视图里直接写英文源串作键，交给 IDE 翻译；代码拼串走 `i18n::t(key, args)`，用 `fs.plugin_dir`（`wit/fs.wit:20`，无需 fs 授权）+ `read_file` 读自己的目录，按「精确 tag → 语言 → 变体」回落（`manifest.mdx:142`）；禁止把 `format!` 结果塞进 label 期待被翻译。
- **依赖**：US-E2-01、US-E5-01..03（文案键）。
- **规模**：S
- **映射**：U2、U8（本册范围）

### US-E6-03 权限最小化与可读错误提示
- **故事句**：作为谨慎的用户，我希望插件只申请必需权限，并在缺权限或出错时告诉我缺什么、去哪里开，以便我敢装也能自己修。
- **验收标准**：
  - Given 全新安装，Then 弹窗里 required 只有 2 项（`project.read`、`editor.read`，均 Low），其余是可单独拒绝的 optional（`docs/en/plugins/capabilities.mdx:10-11,22`）。
  - Given 用户拒绝 `clipboard`，When 执行 US-E5-01/02/03，Then 命令不崩：结果写入 `fs.plugin_dir/out/`，通知文件路径并提示可在 Settings › Plugins 授予 `clipboard`。
  - Given 拒绝 `storage`，Then 索引只驻内存，功能可用，重启后首次刷新变慢，无 panic（`wit/host.wit:187-190`）。
  - Given 拒绝 `events`，Then 不订阅增量事件，刷新按钮或命令触发重扫，仍能出结果。
  - Given 任一 host 调用返回 `Denied("x")`/`Failed`/`Cancelled`，Then 统一映射成「动作 + 原因 + 缺哪个权限/怎么开 + 可否重试」的三段式文案，含 Settings › Plugins 路径与设置键名。
- **技术要点**：
  ```toml
  [capabilities]
  required = ["project.read", "editor.read"]
  optional = ["clipboard", "storage", "events", "clipboard.read", "net:ask"]
  ```
  `Error::Denied` 不终止插件，插件继续运行并降级（`capabilities.mdx:6`）；required 的覆盖匹配见 `crates/ide-extension/src/capability.rs:156-174`；E4 的请求另需按 host 声明 `net:<host>`/`net.http:<host>`，明文到非 loopback 必须写死（`crates/ide-plugin/src/types.rs:565-573`）；通知与状态栏无需权限（`capabilities.mdx:14`、`wit/host.wit:95`）。
- **依赖**：横切全部故事；E4 的 `net:*` 见 US-E4-01。
- **规模**：M
- **映射**：横切 U1–U12

# 第 5~8 章：小步交付计划 · 非功能需求 · 旧缺陷不继承 · 未决问题

本章只依赖 `_brief.md` 的硬约束与 §4 的故事编号，不重复故事正文。

## 5. 小步交付计划

原则（全部迭代共同遵守）：

1. **每迭代结束都能装进 openidea 当场演示**：宁可降范围，不留「下个迭代才看得见」的半成品。
2. **I0 只打通一条最窄链路**：单文件识别 `@GetMapping` 并复制 URL；不建索引、不做树、不读配置文件。
3. **任何故事只在一个迭代内交付完成**，不跨迭代拆半个故事；拆不动就砍验收范围，不挪迭代。
4. **迭代内先写通、再用例固化**：先用最小可见路径接通（可临时硬编码示例数据），再用 MockHost 表驱动用例补齐并合入。
5. 顺序按依赖推进：索引 → 视图 → 复制/详情 → 请求 → 搜索 → 发布。

### I0 单文件识别 + 复制 URL

- 目标：打开一个 Spring Controller 文件，识别 `@GetMapping`，直接把完整 URL 复制到剪贴板。
- 包含故事：US-E1-01、US-E3-01
- 可演示成果：① 本地加载插件目录并激活；② 打开含 `@GetMapping("/users/{id}")` 的 Java 文件，光标停在方法上；③ 执行「Copy Full URL」，剪贴板得到 `http://localhost:8080/users/{id}`。
- 验证方式：MockHost 单测（`find_files` 喂一个文件 → `run_command` 断言剪贴板文本）+ 沙盒手工装插件跑一遍。
- 出口条件 DoD：解析只读文件文本、零 PSI/`textDocument/*` 调用；`plugin.toml` 通过 manifest 校验；单测绿；演示三步可复现。

### I1 全项目索引 + 路径归一化 + 配置推导

- 目标：从「单文件」扩到「全项目索引」，类级 × 方法级路径拼接归一化，并从 `application[-profile].properties|yml` 推导 base URL。
- 包含故事：US-E1-02、US-E1-03、US-E1-05
- 可演示成果：① 在含 3 个 Controller 的示例工程执行「重建索引」，进度条可见；② 查看索引摘要（总条数 + 按模块分布）；③ 对类级 `@RequestMapping("/api/v1")` + 方法级 `@GetMapping("/{id}/")` 的方法复制 URL，得到 `/api/v1/{id}`（无 `//`）；④ `application.yml` 写 `server.port: 8081` 后同一方法复制出 `http://localhost:8081/api/v1/{id}`。
- 验证方式：MockHost（脚本化 `documentIndexer` 回调 + 断言 `storage` 快照）+ 沙盒手工。
- 出口条件 DoD：一万文件首次索引 ≤60 s 且可取消；300 s 闲置回收后从 `storage` 重建不重扫（`wit/host.wit:187-190`）；归一化表驱动用例 ≥20 条全绿。

### I2 Services 树可见可展开可跳转

- 目标：把索引渲染成模块 → 服务 → 方法的 Services 树，能展开、能双击跳源码。
- 包含故事：US-E2-01、US-E2-02、US-E2-03、US-E2-04
- 可演示成果：① 打开 Services 视图，根节点显示「Found N services」；② 逐级展开模块与 Controller，方法节点带 GET/POST 图标；③ 双击某个方法节点跳到声明行。
- 验证方式：MockHost（喂索引 → 断言 `views` body 为扁平列表 + `parent` 关系）+ 沙盒手工。
- 出口条件 DoD：展开事件重渲染 ≤5 s（`crates/ide-plugin/src/types.rs:447-448`）；渲染只读内存快照、不触发解析；根节点计数与索引条数逐一相等；双击与右键 Jump to Source 落到同一位置（US-E2-03）；URL 搜索分类带 `*` 通配命中（US-E2-04）。

### I3 URL 带端口/前缀 + 参数与 Body 骨架

- 目标：复制的 URL 反映真实 `server.port` 与 context-path，并把参数与 body 骨架一次拿到。
- 包含故事：US-E3-02、US-E3-03、US-E3-04
- 可演示成果：① 示例工程 `application-dev.yml` 写 `server.port: 8081`、`server.servlet.context-path: /gateway`，profile 切到 dev；② 树节点右键「Copy Full Url」得到 `http://localhost:8081/gateway/api/v1/users/{id}`；③ 详情面板看到 RequestParams 键值与 RequestBody JSON 骨架。
- 验证方式：MockHost（喂配置文件文本 → 断言端口与前缀）+ 沙盒手工。
- 出口条件 DoD：`application-<profile>` 优先于 `application` 且有单测；context-path 首尾斜杠归一化；`@RequestBody` 字段不进 query 且只取唯一一处；树节点右键聚合菜单三项均可用（US-E3-04）。

### I4 请求面板发出一发并看响应 + 权限降级

- 目标：不离开 IDE 发出一次 HTTP 请求并查看、复制响应。
- 包含故事：US-E4-01、US-E4-02、US-E4-03、US-E6-03
- 可演示成果：① 详情面板点「在请求面板中打开」，表单预填 method / URL / headers / body；② 把 `{id}` 改成 42，点 Send；③ 响应 tab 出现状态行 + 美化后的 JSON，点「Copy response」拿到全文。
- 验证方式：MockHost（脚本化 200/text、超时、403、17 MiB 超限四种响应）+ 沙盒手工。
- 出口条件 DoD：缺 `net.http:<host>` 时给出「需在 manifest 授予」的文案而非静默失败；`timeout-ms` 显式传入且 ≤30 s（`wit/io.wit:65-66`）；>16 MiB 有专门文案；响应不落盘、不进 `storage`。

### I5 Kotlin/JAX-RS 解析 + 类→JSON 三条命令

- 目标：把 Kotlin 与 JAX-RS 源码纳入同一套解析，并把类变成 JSON / 压缩 JSON / Bulk Value 三条命令一次交付。
- 包含故事：US-E1-04、US-E5-01、US-E5-02、US-E5-03
- 可演示成果：① 打开 Kotlin 资源类 `@RequestMapping(value = ["/k"], method = [RequestMethod.POST])` 与 JAX-RS 类 `@Path("/api")`，树里各出现一组路由；② 光标停在 `UserDto` 类体内执行「Convert to JSON」，剪贴板得到缩进 JSON；③ 同一类依次执行压缩版与 Bulk Value，分别得到单行 JSON 与 `id:1`／`name:demoData` 两行。
- 验证方式：MockHost（喂 Kotlin/JAX-RS 文本 → 断言路由；喂含嵌套与自关联的类 → 断言三档输出）+ 沙盒手工。
- 出口条件 DoD：Kotlin/JAX-RS 与 Java 同一套规则表，`@get:`/全限定名/`value = [..]` 各有用例；三档输出共用 `model::build`，占位值表单一来源；自关联连续两次输出逐字节相同。

### I6 可送审的发布件

- 目标：本地化、单一版本源、全量回归，产出可交付的插件包。
- 包含故事：US-E6-01、US-E6-02
- 可演示成果：① 切到 en-US 重启，Services 视图与命令标题全英文、无裸中文；② `plugin.toml` 与 `Cargo.toml` 版本一致，校验脚本通过；③ 22 个故事的 MockHost 用例一键跑完全绿。
- 验证方式：MockHost 全量回归 + 沙盒双语言冒烟。
- 出口条件 DoD：`locales/zh-CN.json` 与 `en-US.json` 键集合 diff 为空；无 `format!` 裸文案进 UI；README 含安装与三步演示。

## 6. 非功能需求

| 维度 | 可验收指标 | 验证手段 |
| --- | --- | --- |
| 性能 · 索引预算 | 单文件文本解析 ≤10 ms；一万文件首次全量索引 ≤60 s（`tasks.begin` 内，每 64 文件 `report` 并查 `is_cancelled`）；增量回调单次 ≤50 ms；`activate` 冷启动只读 `storage` 做差分，≤1 s、不扫盘 | 合成 1 万个 `.java` 计时；MockHost 断言 activate 期间无 `find_files` |
| 性能 · endpoints | `endpoints(ctx)` 墙钟 ≤2 s（硬预算 `crates/ide-host/src/plugins/endpoints.rs:99`），只读内存快照不解析；产出 ≤1 万条、元数据 ≤4 MiB，超限按模块截断并记日志 | MockHost 计时；1.2 万条路由负向用例断言截断 |
| 性能 · 命令/视图 | `run_command`、视图渲染、`expand` 事件各 ≤5 s（`crates/ide-plugin/src/types.rs:447-448`）；只允许「读快照 + 拼字符串」，禁网络与全量扫描；长任务一律 `tasks.begin` | 单测断言命令路径无 `net.fetch`；沙盒抓 timings |
| 权限最小化 | capabilities 只含 `clipboard.read`、`clipboard.write`、`storage`、`ui`、`views/endpoints/details/search` 与 `net:ask`；不写 `net:<host>` 通配、不要写文件权限；明文 HTTP 仅当用户显式加 `net.http:<host>`（`wit/io.wit:46-59`） | 对 `plugin.toml` 跑白名单断言脚本 |
| 线程与预算 | 解析只出现在 `documentIndexer` 回调与 task 线程；渲染/展开零解析；任何超预算任务返回结构化「未完成」而非挂死；300 s 闲置回收后重进功能不降级 | 单测断言调用链无解析器；沙盒等 300 s 回收再操作 |
| 本地化 | `zh-CN`/`en-US` 键集合 100% 对齐；所有用户可见字符串来自 `locales`，`format!` 拼接后也必须查表；缺键使 CI 失败 | 键集合 diff 脚本 + 双语言冒烟 |
| 单测覆盖 | 22 个故事每个 ≥1 个 MockHost 用例；解析规则表 ≥60 条；核心 crate 行覆盖 ≥70%；用户输入路径无 `unwrap()`/`panic!` | `cargo llvm-cov` + review checklist |
| 错误提示 | 6 类错误（注解值解析不出、文件 >16 MiB、请求超时、权限不足、响应 >16 MiB、endpoints 整批作废）各有固定文案键 + 指向 `file:line` 的定位；禁止吞异常返回空 | 每类一个负向单测断言文案键 |

## 7. 明确不继承的旧缺陷

| 缺陷 | 旧代码位置(file:line) | 本方案如何规避 |
| --- | --- | --- |
| EDT 上全量扫描并重建树 | `restful-toolkit-fix/src/main/java/com/zhaow/restful/navigator/RestServicesNavigator.java:177` 在 dispatch 线程调 `myStructure.update()`，进而 `RestServiceStructure.java:122` → `RestServiceProjectsManager.java:68` 的 `runReadActionInSmartMode` 全项目扫描 | 解析只在 `documentIndexer` 回调与 `tasks.begin` 任务线程；视图渲染只读内存索引快照并受 5 s 预算约束（`crates/ide-plugin/src/types.rs:447-448`），`expand` 事件不触发解析 |
| 后台线程直接改 UI | `.../navigator/RestServiceDetail.java:162` 在 `Task.Backgroundable.run`（L124 起的后台线程）里调 `addResponseTabPanel` 改 Swing 组件 | 插件侧无 UI 句柄：命令返回结构化数据，响应由视图层渲染只读 tab；网络结果只写 content tab |
| 通配 pattern 与 path 传反 | `.../navigation/action/GotoRequestMappingModel.java:151` 写成 `pathMatcher.match(popupItem, userPattern)`，而签名是 `match(pattern, path)`（`.../common/spring/AntPathMatcher.java:110`） | 自写 matcher 只暴露 `match(pattern, path)` 一个入口，参数名带语义；`{id}`/`*`/`**` 表驱动用例在 I5 固化 |
| static 可变自关联计数 | `.../common/PsiClassHelper.java:23` 的 `private static int autoCorrelationCount=0`，L152-155 只增不减（`KtClassHelper.java:22` 同样一份） | 递归深度用调用栈内的不可变参数逐层传递（纯函数），无跨调用静态状态；同一 DTO 连续转两次结果必须逐字节相同，作为对拍用例 |
| 请求无超时 | `.../common/RequestHelper.java:49`（`get`，`post`/`put`/`delete` L81/L115 同）用 `HttpClients.createDefault()`，未设 connect/socket 超时 | 请求统一走 `net.fetch` 并显式传 `timeout-ms`，上限 30 s 且被本次调用剩余预算截断（`wit/io.wit:65-66`）；UI 提供取消，超时给固定文案键 |
| 版本号双源不一致 | `restful-toolkit-fix/build.gradle.kts:52` 写 `version = "2.0.8"`，而 `src/main/resources/META-INF/plugin.xml:4` 写 `<version>2.0.7</version>` | `plugin.toml` 的 `version` 为唯一来源，`Cargo.toml` 由校验脚本比对，不一致则 CI 失败；发布产物版本从 `plugin.toml` 读出 |

## 8. 未决问题

1. 内网明文 HTTP 主机（如 `10.0.3.7`）怎么授信：改宿主让 `net:ask` 覆盖明文（`crates/ide-plugin/src/types.rs:567-576`、`crates/ide-extension/src/capability.rs:155-181`），还是要求用户手写 `net.http:<host>`。**影响 US-E4-02**；需 openidea 平台负责人拍板。
2. `[[contributes.endpoints]]` 记录缺 group/module 字段（`wit/endpoints.wit:9-20`）：是否加字段，以决定模块分组能否共享给 Endpoints 搜索。**影响 US-E2-01、US-E5-01**；需 wit/协议维护者拍板。
3. `wit/io.wit:69-74` 注释写 body ≤8 MiB，实现按 16 MiB 全量缓冲（`crates/ide-host/src/plugins/api.rs:2466-2471`）：以哪个为对外承诺，决定「超限」文案与截断策略。**影响 US-E4-03**；需宿主 API 维护者拍板。
4. 设置只有 `settings.get`、没有写回（`wit/views.wit:196-201`）：端口/前缀的用户覆盖是走 IDE 设置页还是插件内命令。**影响 US-E3-02**；需产品负责人拍板。
5. 插件无法调用内建 HTTP Client 动作（`src/features/http-client/generatedRequests.ts:9,45-58`）：决定「在请求面板中打开」是自建表单还是唤起内建客户端。**影响 US-E4-01**；需宿主集成负责人拍板。

## 九、实现状态与偏差

> 统计口径：代码在 `openidea-restfultoolkit` 仓库，下文 `src/…:函数` 相对该仓库；`cargo test` 135 通过（含集成测试 `tests/scan_test.rs`）；`cargo build --release --target wasm32-wasip2` 成功，产物 `dist/plugin.wasm` 503 284 字节；清单计数按 `plugin.toml` 实测。状态三档：**已实现**＝主链路可运行且有单测；**部分实现**＝主链路可运行但验收标准有子项未落地或口径不同；**未实现**＝无对应代码。

### 9.1 总体状态

| 指标 | 值 |
| --- | --- |
| 模块数 | 11（`src/` 下除入口 `lib.rs` 外的 11 个模块：config / format / http / index / json_gen / params / scan / tree / url / view / detail） |
| 测试数 | 135 通过（`cargo test`，含 `tests/scan_test.rs`） |
| wasm 大小 | 503 284 字节（`dist/plugin.wasm`，wasm32-wasip2 release 构建） |
| 命令数 | 14（`[[contributes.commands]]`：copy-full-url、copy-query-param、copy-request-body、refresh-index、search-routes、send-request、convert-json、convert-json-compressed、convert-bulk-value、copy-url、open-source、restful-pick、test-parse、copy-many-urls） |
| 视图数 | 1（`restful-routes`，`location = "services"`，`detail = true`） |
| 菜单数 | 9（`editor.context` 6 项 + `viewItem.context` 3 项） |
| 贡献点 | 7 类：commands 14、menus 9、keymaps 3、views 1、endpoints 1（`files = ["**/*.java", "**/*.kt"]`）、searchCategories 1（`routes`）、settings 1（1 页 + `Test parse` action） |
| 审计判定 | 通过 10 / 部分 12 / 未实现 0（22 条均有实现，差距为规格细节；对应下表「已实现」10 条与「部分实现」12 条） |

### 9.2 逐故事状态

| 故事 | 状态 | 证据(文件:函数) | 偏差 |
| --- | --- | --- | --- |
| US-E1-01 | 部分实现 | `src/scan.rs:scan_file`、`src/scan.rs:class_base_path`、`tests/scan_test.rs:class_and_method_mapping_are_paired` | 注解值只认字面量，常量/拼接/`${…}` 置 `unresolved` 并保留原文；未解析计数只在 `test-parse` 通知里，未上树/索引状态行 |
| US-E1-02 | 已实现 | `src/lib.rs:RestfulToolkit::handle_events` / `apply_file_changes`、`src/index.rs:RouteIndex::apply_changes` | `files-changed` 增量已接（新增/修改只重扫该文件、删除移除条目）；首次全量仍靠手动 `refresh-index`，`find_files` 每 glob 上限 2000 |
| US-E1-03 | 已实现 | `src/url.rs:join_url`、`src/url.rs:normalize_path` | 无（`{id:[0-9]+}` 内字符原样保留） |
| US-E1-04 | 已实现 | `src/scan.rs:scan_kotlin`、`src/scan.rs:scan_jaxrs`、`src/scan.rs:scan_any` | `endpoints` 输出统一写 `framework = "Spring"`，未保留 JAX-RS 标记（`src/lib.rs:EndpointsProvider::endpoints`） |
| US-E1-05 | 已实现 | `src/lib.rs:base_url`、`src/config.rs:read_server_port` / `read_context_path` / `read_active_profile` | 设置项 `activeProfile` 优先于 `spring.profiles.active`；配置收集每次每 glob 最多 32 个文件（`src/lib.rs:base_url`） |
| US-E2-01 | 部分实现 | `src/tree.rs:flatten`、`src/view.rs:Views::render` / `handle` | 方法节点仍用 `[GET]` / `[POST]` 文本前缀（宿主无动词图标）；根计数与个别节点文案硬编码；节点上限 5000（`src/tree.rs:MAX_NODES`），无懒加载 |
| US-E2-02 | 部分实现 | `src/detail.rs:tabs_for`、`src/lib.rs:Details::detail` | 详情按「视图 id\|节点 key」缓存在实例内存，`refresh-index` 才清空；tab 文案个别硬编码；`storage` 被拒时详情仍读 `storage` |
| US-E2-03 | 部分实现 | `src/view.rs:activate` / `open` / `open_node` | 控制器节点跳该文件首个方法行，而非类声明行；列固定 0；文件不存在时报「源码位置已失效」 |
| US-E2-04 | 已实现 | `src/lib.rs:SearchProvider::search`、`src/lib.rs:wildcard` / `path_part` | 无（空查询已按模块→路径→方法排序，完整 URL 会先剥本项目 context-path） |
| US-E3-01 | 部分实现 | `src/lib.rs:resolve`、`src/url.rs:join_url`、`src/lib.rs:copy_text` | 命令未加 `editorLangId` 的 `when`（只有菜单项带）；`run_command` 的 `arg` 未使用，「相对 URL」未实现 |
| US-E3-02 | 部分实现 | `src/lib.rs:copy_query`、`src/params.rs:parse` / `query_string` | 命令未加语言 `when`；query 占位值为 `0` / `false` / 空串（`query_default`），与 US-E5 的 JSON 占位值分属两种用途 |
| US-E3-03 | 部分实现 | `src/lib.rs:copy_body`、`src/json_gen.rs:json_for_with` / `base_default` | 命令未加语言 `when`；占位值全部取自 US-E5；无 `@RequestBody` 时报错且不动剪贴板 |
| US-E3-04 | 已实现 | `src/lib.rs:pick_command` / `copy_node_urls` / `copy_many_urls`、`plugin.toml` 的 `restful-pick` keymap | 无（Alt+Shift+R 绑 `restful-pick` 聚合 pick；多选走 `copy-many-urls` + `pick_many`，按 `\n\n` 拼接） |
| US-E4-01 | 部分实现 | `src/lib.rs:send`、`src/lib.rs:explain_fetch`、`src/lib.rs:request_from_endpoint` | 无独立请求表单面板、未用 `net:ask`；超时取自设置未夹 30 s；headers 仅 `extraHeaders`，手输 `METHOD URL` 时 `body = None` |
| US-E4-02 | 已实现 | `src/http.rs:apply_path_vars` / `append_query` / `encode` / `parse_request_input` | 无（路径变量值百分号编码，缺变量在通知里列出并保留占位符；变量取自命令输入的 `k=v` 列表） |
| US-E4-03 | 部分实现 | `src/format.rs:pretty_json` / `truncate`、`src/lib.rs:Details::detail` 追加 Response/Headers tab | 超时未夹 30 s（按设置 `requestTimeoutMs` 原样下发）；响应只留实例内存最近一次，不落盘 |
| US-E5-01 | 部分实现 | `src/lib.rs:convert`（`Convert::Json`）、`src/json_gen.rs:class_json_with` / `fields_of` | 未解析字段写 `null` 但不在通知里列清单；命令未加语言 `when`；复制前按 900 KiB 判长 |
| US-E5-02 | 已实现 | `src/json_gen.rs:to_compressed`、`src/lib.rs:convert`（`Convert::Compressed`） | 无（与缩进版共用 `class_json_with` 内核） |
| US-E5-03 | 已实现 | `src/json_gen.rs:to_bulk_value_full`、`src/lib.rs:line_ending` / `parent_text` | 继承链只认文本 `extends` + 符号查类；默认 CRLF，`lineEnding = lf` 切 LF |
| US-E6-01 | 已实现 | `plugin.toml` 的 `contributes.settings.pages` + `actions`、`src/lib.rs:test_parse` / `json_options` / `line_ending` | 键名是顶层 `maxDepth` / `lineEnding` / `skipSerialVersionUid` 等，与 US-E5 正文的 `json.*` / `bulkValue.*` 不同；未订阅 `settings-changed`，每次现读 |
| US-E6-02 | 部分实现 | `src/lib.rs:tr` / `catalog`、`locales/en.json`、`locales/zh-CN.json` | 英文文件名是 `en.json`（非 `en-US.json`）；树根计数与详情 tab 文案有个别硬编码 |
| US-E6-03 | 部分实现 | `src/lib.rs:denied_hint` / `explain_fetch` / `save_signatures`、`src/lib.rs:apply_file_changes` | `storage` 被拒时索引落内存但详情面板仍读 `storage`；`clipboard` 被拒不落盘 `out/`；optional 为 `net:localhost` / `net.http:localhost` |

### 9.3 偏差清单

1. **未解析路由计数未上状态行**：`test-parse` 通知里有「N unresolved」，树与索引刷新不展示该计数（US-E1-01）。
2. **方法节点仍用 `[GET]` 前缀**：宿主图标集无 HTTP 动词图标，未按方法换图标（US-E2-01）。
3. **控制器节点跳首个方法行**：双击控制器落到该文件最靠前的路由行，而非类声明行（US-E2-03）。
4. **命令未加语言 `when` 限制**：复制/转换命令只在菜单项带 `editorLangId`，从命令面板可直接调用（US-E3-01、US-E3-02、US-E3-03、US-E5-01）。
5. **无独立请求表单面板与 `net:ask`**：发送走命令输入 `METHOD URL`，权限声明 `net:localhost` / `net.http:localhost`，未走询问授权（US-E4-01）。
6. **超时未夹 30 s**：`requestTimeoutMs` 原样下发，未按验收标准的 ≤30 s 截断（US-E4-01、US-E4-03）。
7. **类→JSON 未解析字段不提示**：未解析类型写 `null`，但通知里不列 `类.字段` 清单（US-E5-01）。
8. **个别文案硬编码**：树根计数与详情 tab 标题未全部走 `tr()`（US-E2-01、US-E2-02、US-E6-02）。
9. **`storage` 被拒时详情面板仍读 `storage`**：内存降级只覆盖索引读写，详情未改用内存副本（US-E2-02、US-E6-03）。
10. **未做沙盒端到端**：仅 MockHost 单测（135 通过）+ wasm 构建 + 清单静态校验，未在真实 openidea 里手工跑全链路。

### 9.4 后续待办

1. 补 openidea 沙盒端到端脚本：装 `dist/plugin.wasm` → Services 树展开 → 复制 URL / Query / Body → 发送并看响应 → 三条类转 JSON 命令，纳入发布前检查。
2. 补覆盖率：`cargo llvm-cov` 跑通并记录核心模块行覆盖，对齐 §6 的 ≥70% 指标。
3. 做独立请求表单面板：headers/body 可编辑，按 host 走 `net:ask`，超时夹到 30 s 以内。
4. 接 IDE 内建 HTTP Client（或按其请求格式导出），替换现在的命令式发送。
5. 收口规格细节：未解析计数上状态行、类→JSON 未解析字段清单、命令语言 `when`、文案全部走 `tr()`、`storage` 被拒时详情改用内存索引。
