# services

职责：

- `acp`：启动本地 `stdio` ACP agent、维护 session 生命周期并投影消息流
- `application`：扫描已安装应用、缓存索引并执行应用启动
- `matcher`：根据输入上下文筛选并排序动作
- `executor`：执行动作并返回结构化结果
- `file_search`：基于当前 workspace 做模糊文件搜索
- `ocr`：定义 OCR provider 抽象、macOS Vision provider、OpenAI 兼容多模态 provider，以及交互式截图 OCR 所需的临时文件与命中点模型
- `rag`：维护本地 LanceDB 向量索引，负责目录扫描、文件监听、文本切分、embedding 调用和增量重建
- `selection`：读取当前活跃应用复制动作产生的选中文本，并避免把旧剪贴板内容误判成新选区
- `public_skills`：扫描 `~/.agents/skills`，解析 `SKILL.md` frontmatter，并构建目录树

边界：

- `matcher` 与 `executor` 优先保持纯逻辑，便于单元测试
- `application` 当前刻意只覆盖 macOS `.app` bundle：扫描 `/Applications`、`/System/Applications` 和 `~/Applications`
- `application` 对每个 bundle 优先读取 macOS metadata 里的本地化显示名，并把 bundle 目录名保留为别名参与匹配，解决 `WeChat.app` / `DingTalk.app` 这类中文名称搜索问题
- `application` 在启动后后台预热索引，并维持常驻内存快照；查询阶段只打内存，不同步触发扫描
- `application` 后台每 5 分钟定时刷新一次索引；快照过旧时只异步补刷新，旧快照继续服务，避免把刷新抖动带回输入链路
- `application` 启动应用统一走系统 `open <bundle-path>`；它返回普通 `ExecutionResult`，但不复用 action 协议去表达“应用对象”
- `matcher` 只返回真实可执行的 `/` 命令，并把评分细节留在服务内部，不把“命中原因”暴露到 UI；`json_pretty_print` 额外支持 `/format`、`/fmt` 和兼容别名 `/json` 的“命令 + JSON 载荷”输入形态
- `executor` 执行 `json_pretty_print` 时会优先剥离 `/format` / `/fmt` / `/json` 前缀，只把后续 JSON 载荷送进格式化逻辑
- `matcher` / `executor` 对 `markdown_render` 额外支持 `/md` 和长别名 `/markdown`；命中后直接返回原始 Markdown 文本，并在 `structured_payload.render=markdown` 上声明前端应按 Markdown 渲染
- `matcher` / `executor` 对 `base64_text` 额外支持 `/base64 <payload>`；执行时会先尝试把载荷识别为 UTF-8 Base64 文本，命中则解码，否则编码
- `matcher` / `executor` 当前额外内建一组纯文本处理 slash 动作：`/upper`、`/title`、`/lower`、`/camel`、`/snake`、`/trim`、`/unique`、`/sort`、`/words`、`/lines`
- `camel_case_text` / `snake_case_text` 逐行做命名风格转换；单词拆分会同时识别空白、常见分隔符和 `camelCase` / `HTTPServer` 这类大小写边界
- `unique_lines` / `sort_lines` 按行处理文本：`/unique` 保留首次出现的行，`/sort` 做字典序排序；它们不会顺手裁剪空白，清理空白仍由 `/trim` 负责
- `file_search` 按操作系统选择搜索后端；当前 macOS 优先 workspace 范围内的 Spotlight，失败或无结果时回退到 `ignore` + `skim` matcher 的本地索引
- `acp` 当前只支持本地 `stdio` ACP transport，不声明文件或终端 capability；但 session 创建/恢复时会把全局 MCP server 配置透传给 ACP agent
- `acp` 当前不会把自己实现成 MCP client/bridge；如果 agent 支持 ACP `mcp_servers`，就由 agent 自己连接这些 MCP server
- `acp` 会持久化 live session 快照，并在启动时尝试使用 agent 的 `session/load` 恢复；不支持或失败时只记录恢复提示
- `public_skills` 是只读服务：只返回 skill meta、目录/文件统计和层级树，不读取普通文件内容
- 真正的系统能力接入通过基础设施层或前端 effect 完成
- 当前 workspace 属于运行时状态：启动默认回到 `HOME`，配置层只持久化最近 3 个目录用于快速切换
- `ocr` 现在支持两类 provider：macOS 本地 `Vision`，以及通过 OpenAI 兼容 `chat/completions` 接口发送图文输入的远程多模态模型
- OCR provider 选择、OpenAI 风格 `base_url/api_key/model` 和 provider 协议声明来自配置层；当前 OCR 只消费 `openai_chat + supports_multimodal` 这条组合。保存设置时会做基本校验，启动阶段遇到坏配置则降级为明确不可用状态
- 交互式截图本身仍然只在 macOS 下实现；远程 provider 目前只是替换“识别器”，没有顺带把截图能力跨平台化
- `rag` 只消费配置层里协议为 `openai_embedding` 的 provider；它只会扫描后缀为 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc` 的文本文件，再要求内容是可读 UTF-8、大小不超过 50 MB 且不含 NUL 字节；Markdown 类文件使用 `MarkdownSplitter`，其余文本文件使用 `TextSplitter`，最后把向量和元数据写入 LanceDB
- `rag` 通过 `notify` 监听选中目录；文件变化时先删旧向量，再写新向量，目录级变更则直接回退到全量重建
