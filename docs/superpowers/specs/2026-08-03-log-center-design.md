# CC Switch 日志中心设计

## 背景

CC Switch 当前通过 `tauri-plugin-log` 将运行日志写入 `<app_config_dir>/logs/cc-switch.log`，单文件上限 20 MiB，并保留 4 个轮转归档。日志级别由 SQLite 主库中的 `log_config` 控制。

代理请求的统计摘要已经写入主库 `cc-switch.db` 的 `proxy_request_logs` 表，并在“使用统计”页面展示状态码、Token、费用和耗时。该表不保存完整请求管线，也不适合承接高频 SSE 详情。当前代理日志对请求体、响应体和 SSE 内容主要记录大小、哈希或事件类型，正文通常明确标记为 `content omitted`。

本设计新增独立日志中心，并将可查询的运行日志、请求/响应详情和 SSE 事件保存到本地 SQLite。高频诊断数据与供应商、MCP、设置等业务数据隔离，避免影响主库备份、同步和日常读写。

## 目标

- 提供独立“日志中心”页面，包含“请求追踪”和“运行日志”两个页签。
- 记录代理请求的客户端输入、转换结果、每次上游尝试、重试、上游响应、最终响应和 SSE 事件。
- 默认在落入队列前脱敏，避免把认证信息和明显敏感大字段写入磁盘。
- 使用批量写入和 WAL 降低磁盘写入频率，日志处理不得阻塞代理请求。
- 日志保留 3 天；请求追踪逻辑存储上限 300 MiB，运行日志逻辑存储上限 50 MiB。
- 页面打开时实时追加摘要，并允许暂停；历史记录可筛选、分页和按需加载详情。
- SQLite 不可用时保留最小文件兜底，保证启动、崩溃和日志数据库故障仍可诊断。

## 非目标

- 不提供 MySQL 或 PostgreSQL 后端。单机桌面应用的单写入者场景优先使用 SQLite。
- 不构建跨设备集中日志平台，也不上传日志到远端服务。
- 不把日志数据库纳入 WebDAV、S3、配置导出或数据库备份。
- 不保存底层网络任意分块；SSE 按协议事件聚合后保存。
- 不保证异常崩溃前最后一秒内的低优先级详情完整落盘。
- 不在本功能中重构现有使用统计页面；只通过标识符建立关联。

## 总体架构

新增 `<app_config_dir>/cc-switch-logs.db`，专门保存高频、短生命周期日志。现有 `cc-switch.db` 继续保存供应商、MCP、设置及 `proxy_request_logs` 使用统计摘要。

日志数据流如下：

1. 代理管线和运行日志系统产生结构化事件。
2. 事件在进入队列前完成 Header 过滤、JSON 递归脱敏和大字段处理。
3. 事件进入有界、带优先级的异步队列；生产方只执行非阻塞发送。
4. 单一后台写入任务聚合事件，并以批量事务写入 `cc-switch-logs.db`。
5. 后端查询命令从日志库读取摘要和详情；需要使用统计时按 `usage_request_id` 查询主库，不执行跨库 SQL join。
6. 后端只向前端推送新增或更新的 trace 摘要通知，正文由页面按需查询。

运行日志通过 `tauri-plugin-log` v2 的自定义 Dispatch 目标送入同一有界队列。Dispatch 目标只复制结构化记录并尝试入队，不直接访问 SQLite。保留的文件目标缩减为故障兜底目标，不再接收普通请求详情。

请求详情不从格式化文本日志反向解析。代理处理代码在明确的管线阶段直接产生结构化 trace 事件，以保留重试次数、阶段、相对时间和正文类型。

## 数据模型

### `request_traces`

每个客户端代理请求一行，承担列表查询：

- `trace_id TEXT PRIMARY KEY`：请求开始时生成的内部稳定标识。
- `usage_request_id TEXT NULL`：请求完成并写入 `proxy_request_logs` 后建立的关联。
- `app_type TEXT NOT NULL`
- `method TEXT NOT NULL`
- `path TEXT NOT NULL`：只保存代理本地路径，不保存带凭据的完整目标 URL。
- `request_model TEXT NULL`
- `response_model TEXT NULL`
- `final_provider_id TEXT NULL`
- `status_code INTEGER NULL`
- `is_streaming INTEGER NOT NULL`
- `attempt_count INTEGER NOT NULL DEFAULT 0`
- `started_at INTEGER NOT NULL`：Unix 毫秒。
- `completed_at INTEGER NULL`
- `duration_ms INTEGER NULL`
- `outcome TEXT NOT NULL`：`in_progress`、`success`、`error`、`cancelled`。
- `partial INTEGER NOT NULL DEFAULT 0`
- `dropped_event_count INTEGER NOT NULL DEFAULT 0`
- `stored_bytes INTEGER NOT NULL DEFAULT 0`

索引覆盖 `started_at DESC`、`app_type + started_at`、`status_code + started_at`、`final_provider_id + started_at` 和 `usage_request_id`。模型和自由文本搜索第一版使用带前缀限制的普通查询，不引入 FTS。

`request_traces.stored_bytes` 是该 trace 的摘要、事件和 payload 实际持久化内容之和；写入和级联删除时在同一事务内增减。容量控制以此值为准，而不是只计算正文 BLOB。

### `trace_events`

保存有序管线事件：

- `event_id INTEGER PRIMARY KEY AUTOINCREMENT`
- `trace_id TEXT NOT NULL`
- `sequence INTEGER NOT NULL`
- `occurred_at INTEGER NOT NULL`
- `offset_ms INTEGER NOT NULL`
- `stage TEXT NOT NULL`：`client_request`、`transform`、`upstream_attempt`、`upstream_response`、`client_response`、`stream`、`complete`。
- `kind TEXT NOT NULL`
- `attempt_no INTEGER NULL`
- `provider_id TEXT NULL`
- `status_code INTEGER NULL`
- `summary TEXT NULL`
- `payload_id INTEGER NULL`

`(trace_id, sequence)` 唯一，删除 trace 时级联删除事件。

### `trace_payloads`

正文与 SSE 数据独立存储，以便列表和时间线查询不触碰大字段：

- `payload_id INTEGER PRIMARY KEY AUTOINCREMENT`
- `content_type TEXT NOT NULL`
- `encoding TEXT NOT NULL`：`identity` 或 `zstd`。
- `body BLOB NOT NULL`
- `original_bytes INTEGER NOT NULL`
- `stored_bytes INTEGER NOT NULL`
- `sha256 TEXT NOT NULL`
- `truncated INTEGER NOT NULL DEFAULT 0`

大于 4 KiB 的正文使用 zstd 压缩。单个脱敏后 payload 默认最多保存 1 MiB；超出部分截断并设置 `truncated`。SSE 不按网络 chunk 建行，而是解析为协议事件后聚合；每批最多 256 KiB 或 1 秒，先到者触发提交。

### `runtime_logs`

- `log_id INTEGER PRIMARY KEY AUTOINCREMENT`
- `occurred_at INTEGER NOT NULL`
- `level TEXT NOT NULL`
- `target TEXT NOT NULL`
- `message TEXT NOT NULL`
- `fields_json TEXT NULL`
- `stored_bytes INTEGER NOT NULL`

索引覆盖 `occurred_at DESC`、`level + occurred_at` 和 `target + occurred_at`。

### `log_metadata`

保存日志 schema 版本、最近清理时间、分类逻辑字节数和健康状态。逻辑字节计数在同一事务内更新，用于快速判断容量，不在每次写入时扫描全表。

## 脱敏规则

脱敏发生在事件进入内存队列之前，原始敏感正文不进入日志后台任务。

- `authorization`、`proxy-authorization`、`cookie`、`set-cookie`、`x-api-key` 以及名称匹配 `token`、`secret`、`password`、`credential`、`api[_-]?key` 的 Header 或字段替换为 `[REDACTED]`。
- URL 只保留协议、主机、端口和安全路径；query、userinfo 及无法确认安全的路径内容移除。
- JSON 对象递归按键名脱敏；字符串中的常见 Bearer Token 和 API Key 形式做二次兜底替换。
- `data:` URL、图片、音频、文件和疑似 Base64 大字段替换为包含 MIME、原始长度和哈希的占位对象。
- 无法解析的文本执行通用字符串脱敏后保存；二进制正文只保存类型、长度和哈希，不保存原始字节。
- 导出时再次执行同一套脱敏器，防止早期版本或遗漏规则产生的历史数据直接外泄。

页面只展示已经脱敏的内容，不提供“查看原始凭据”能力。

## 写入与磁盘策略

- 日志数据库使用 WAL，`synchronous=NORMAL`，启用 foreign keys 和 incremental auto-vacuum。
- 运行日志每 250 毫秒或累计 100 条提交一次事务。
- SSE 每 1 秒或累计 256 KiB 提交一次，请求结束时提交剩余事件。
- 非流式请求在响应完成时批量提交正文和结束摘要。
- 写入队列有固定上限，不随日志量无限增长；代理请求线程和异步任务不等待 SQLite。
- 正常退出时仅在有限时间内冲刷高优先级事件，不因日志任务长期阻塞退出。

该策略会产生持续但合并后的磁盘写入。应用异常崩溃时，允许损失最后约 1 秒的低优先级 SSE 详情，以换取更少事务和更低写放大。

## 保留与容量

- 启动日志服务后执行一次维护，此后按固定低频周期执行。
- 删除 `started_at` 早于当前时间 3 天的请求 trace，删除 `occurred_at` 早于当前时间 3 天的运行日志。
- 请求追踪逻辑存储超过 300 MiB 时，从最旧 trace 开始整条删除，直到回到上限以下。
- 运行日志逻辑存储超过 50 MiB 时，从最低时间顺序删除整行，直到回到上限以下。
- 清理事务结束后执行 WAL checkpoint；仅在达到空闲页阈值时执行 incremental vacuum。
- 逻辑容量限制不承诺数据库文件在每一时刻精确等于 350 MiB。WAL、索引和待回收空闲页会造成短期额外占用。

日志库不进入现有数据库备份、导入、WebDAV 和 S3 流程。恢复主库不会覆盖本地日志库。

## 故障与降级

队列事件分为高、中、低优先级：

- 高：请求开始、每次重试、错误、取消、请求结束、`error` 运行日志。
- 中：请求/响应摘要、正文批次、`warn` 和 `info` 运行日志。
- 低：重复 SSE 增量、`debug` 和 `trace` 运行日志。

队列拥堵时先丢弃低优先级运行日志，再丢弃重复 SSE 增量。对应 trace 设置 `partial=1` 并累计 `dropped_event_count`。页面必须显示“详情不完整”，不得把缺失内容表现为上游未发送。

日志库写入失败时：

1. 当前批次失败，不影响代理响应。
2. 后台写入器按指数退避重试打开或写入日志库。
3. 限频后的故障信息写入最小轮转文件。
4. 日志中心显示不可用或降级状态。

日志库损坏时不自动删除。页面提供带二次确认的“重建日志库”命令：关闭连接，将损坏文件重命名为带时间戳的诊断文件，再创建新库。清理失败只记录告警；达到硬性容量上限且无法清理时暂停正文采集，只保留高优先级摘要和错误。

数据库和日志基础设施故障绕过用户配置的普通运行日志级别，直接写入兜底文件；因此用户关闭运行日志后，关键启动、崩溃和数据库故障仍然可诊断。

## 兜底文件

保留小型轮转文件用于以下内容：

- 应用和日志系统启动。
- panic 与崩溃信息。
- 主数据库或日志数据库初始化、迁移、损坏和写入失败。
- 日志后台任务异常退出。

兜底文件不记录请求正文、响应正文、SSE 内容或普通 info/debug/trace 运行日志。现有 `crash.log` 行为继续保留；常规 `cc-switch.log` 仅接收上述兜底事件，当前文件上限 4 MiB，并保留 4 个轮转归档，总量最多 20 MiB。

## 后端接口

新增的 Tauri 命令按职责拆分：

- 查询请求 trace 摘要，支持时间、应用、供应商、模型、状态、流式类型、关键字和游标分页。
- 查询单个 trace 的概览与时间线。
- 分页查询单个 trace 的事件或指定 payload；大正文不随摘要返回。
- 查询运行日志，支持时间、级别、模块、关键字和游标分页。
- 查询日志健康状态、容量和最近清理结果。
- 导出选中 trace 或筛选范围内的脱敏 JSONL。
- 清空请求追踪、运行日志或全部日志。
- 用户确认后重建损坏的日志库。

分页使用稳定的 `(timestamp, id)` 游标，避免实时插入导致页码漂移。所有命令限制最大页大小和最大正文返回量。

新增后端事件只包含 trace 标识、摘要变更类型和健康状态变化。前端收到事件后合并列表摘要或使查询失效，不通过事件总线传输正文。

## 页面设计

主界面标题栏新增日志图标，进入独立 `logs` 视图，沿用现有返回按钮和页面标题模式。

### 请求追踪

桌面宽度采用分栏追踪器：左侧约 38% 为紧凑请求列表，右侧约 62% 为详情。窄窗口中详情替换列表，并提供返回按钮，避免双栏挤压正文。

顶部包含：

- “请求追踪 / 运行日志”页签。
- 关键字、应用、供应商、模型、状态、流式类型和时间筛选。
- 暂停/继续实时追加、刷新、导出、清空等命令按钮。
- 最近 3 天和当前分类容量占用。

请求详情包含：

- 概览：状态、模型映射、供应商、耗时、首字节时间、重试和统计摘要。
- 时间线：按相对时间展示客户端输入、转换、每次上游尝试、响应和结束。
- 客户端请求：脱敏 Header 与请求体。
- 上游尝试：按 attempt 分组展示目标摘要、请求和失败原因。
- 响应：上游响应与最终客户端响应。
- SSE：按协议事件展示顺序、相对时间、事件类型和脱敏 `data`，支持搜索和复制。

长列表和 SSE 事件使用虚拟化。选择 trace 后才加载详情，切换详情时取消过期查询。

### 运行日志

运行日志使用紧凑虚拟列表，展示时间、级别、模块和消息。支持级别、模块、关键字及时间筛选；选中后显示结构化字段。实时追加可暂停，暂停只停止 UI 跟随，不停止后端采集。

### 设置

现有日志启用和级别配置继续控制运行日志。高级设置增加“记录请求详情”总开关；关闭后不创建新的 `request_traces`、`trace_events` 或 `trace_payloads` 记录，但现有 `proxy_request_logs` 使用统计摘要保持原行为。数据库和崩溃兜底日志不受这两个普通日志开关影响。

所有新增用户文案同步更新 `zh`、`zh-TW`、`en` 和 `ja` 四套翻译目录。

## 导出与删除

- 导出格式为 UTF-8 JSONL，包含 schema 版本和导出时间。
- 默认导出当前选中 trace；批量导出必须遵守筛选范围和最大记录数限制。
- 导出前再次脱敏，并标注截断、缺失和丢弃计数。
- 清空操作区分“请求追踪”“运行日志”“全部日志”，均要求确认。
- 删除按数据库事务执行；失败时页面保持原查询结果并显示错误，不进行乐观删除。

## 测试与验证

### Rust 单元与集成测试

- 递归脱敏、Header 过滤、URL 处理、Bearer/API Key 兜底替换。
- Base64、媒体、二进制和超大 payload 的占位与截断。
- SSE 跨 chunk 解析、事件顺序、批量阈值和压缩往返。
- 队列优先级、拥堵丢弃、`partial` 标记和高优先级结束事件保留。
- 日志 schema 初始化、升级、级联删除和游标分页。
- 3 天清理、300/50 MiB 分类上限、WAL checkpoint 和逻辑字节计数。
- 日志库不参与主库备份、导入和云同步。
- 普通响应、流式响应、转换、重试、超时和客户端中断的 trace 阶段。
- 日志库打开、写入或清理失败不改变代理响应。
- Tauri 查询、导出、清空和重建命令的权限与边界限制。

### 前端测试

- 双页签、分栏选择和窄窗口列表/详情切换。
- 筛选、游标加载、正文懒加载和过期请求取消。
- 实时追加暂停/继续、健康状态和不完整详情标记。
- 日志详情搜索、复制、导出与清空确认。
- 所有新增翻译键在四套目录中存在。

### 性能与手工验证

- 模拟大量 SSE 小块，验证不会逐块提交事务、队列保持有界且丢弃可观测。
- 验证页面读取期间写入持续进行，列表和 SSE 长内容不造成明显布局跳动。
- 检查桌面及窄窗口截图，确认工具栏、列表和详情不重叠，长文本不撑破容器。

本机没有 Rust 工具链，且不会为本任务安装。后续实施在本机执行 `pnpm typecheck`、`pnpm test:unit` 和前端构建；`cargo fmt --check`、`cargo clippy`、`cargo test` 及 Rust 编译必须由现有 CI 或具备 Rust 环境的维护者执行。交付说明必须明确记录 Rust 验证未在本机完成，不能将其报告为已通过。

## 验收标准

- 用户能从主界面进入日志中心，并在两个页签间切换。
- 请求列表能实时显示新请求；暂停后列表位置稳定，继续后可获取最新摘要。
- 单个请求能展示客户端请求、转换、全部上游尝试、响应及解析后的 SSE 事件。
- 页面和导出内容不包含已知认证 Header、API Key、Cookie 或原始媒体/Base64 大字段。
- 高频 SSE 不产生逐 chunk SQLite 事务，代理请求不等待日志写入。
- 超过 3 天或分类容量上限的最旧日志自动清理。
- 日志库故障不影响代理服务，并能从兜底文件和页面健康状态发现。
- 日志数据不进入主数据库备份、导出或云同步。
