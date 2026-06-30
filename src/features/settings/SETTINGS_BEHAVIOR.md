# Settings Behavior

## 后端接口

设置页使用这些后端命令：

- `getAppSettings` / `setAppSettings`：读取和写入通用、外观、功能、LLM、OCR、知识库配置。
- `listBuiltinLlmProviderTemplates`：返回只读内置 LLM 供应商模板目录，供设置页展示注册引导、默认接入点和模型拉取入口说明。
- `scanRagSources`：用当前草稿立即触发一次知识库全量扫描，返回 LanceDB 路径和统计结果。
- `getShortcut` / `setShortcut`：读取和更新快捷键。
- `getMcpServers` / `setMcpServers`：读取和更新全局 MCP server 清单。

设置页不解释配置文件路径或格式；持久化细节由 Rust `ConfigStore` 负责。

- 配置文件路径由后端固定为 `dirs::config_dir()/wabity/config.toml`。
- 写入由后端 `safe_write` 原子替换。
- 前端只有在命令成功后才提交新状态。
- 保存失败时保持旧值，不把前端草稿伪装成已落盘状态。

## 保存边界

- 通用 / 外观配置可实时写回。
- 快捷键通过独立命令写回。
- 功能配置作为独立分组写回，覆盖翻译提示词、文档问答系统提示词、翻译 LLM、问答 LLM。
- OCR 有独立保存动作，不和翻译 / 文档问答草稿一起保存。
- 模型、OCR、知识库各自维护草稿；保存某个分组时不能把其它分组的未保存草稿偷偷带进 `config.toml`。
- 保存模型分组时，前端只提交当前 provider / model 草稿和已落盘的翻译、问答、OCR、知识库引用。
- 删除或替换模型后的依赖引用修复必须由 Rust 侧基于最新 LLM 目录归一化，再做跨分组校验。

## 通用和系统集成

- `autoStart` 不只是写 `config.toml`；Rust 侧保存成功后必须同步系统登录启动项，应用启动时还会再做一次配置与系统状态对账。
- `showInDock` 不只是写 `config.toml`；macOS 下 Rust 侧保存成功后必须立即同步应用激活策略和 Dock 图标可见性，应用启动时同样按当前配置恢复。
- 通知配置也实时持久化；设置页只负责开关、摘要粒度和系统设置引导，完成事件判断与系统通知发送仍由 Rust 侧调度。
- 外观设置除了持久化，还必须立即同步前端根节点：`theme` 负责 `data-theme` / `color-scheme`，`fontSize` 负责 `data-font-size`。
- 默认快捷键当前为 launcher=`Alt+Space`、翻译选中文本 / 未选中时 OCR=`Alt+D`、打开历史剪贴板=`Alt+V`。

## 模型配置

- LLM 条目支持多条目录项。
- `Base URL` 由用户显式填写到 API 根路径，通常包含 `/v1`。
- `API key` 允许为空，兼容本地或内网 OpenAI 兼容网关。
- 手工条目可根据当前 `baseUrl + apiKey` 调 `/models` 拉模型列表，并保留手填能力。
- 模板条目以远端目录为准；目录不可用时退回手填。
- 模板只负责供应商默认值和官方入口，不接管用户条目本身。
- 内置模板的默认模型必须显式写在模板元数据里，不能依赖模板数组顺序。
- 模型命中模板目录时，协议、类型、OCR 能力、stateful 能力应按模型元数据自动回填。
- 未知模型才允许手动指定调用方式。
- `supportsMultimodal` 当前只对 `responses` 路径有意义；切到 `chat/completions` 或 Embedding 时固定关闭。
- `supportsStateful` 折叠进“调用方式”选择器；它只控制 launcher 文档问答继续追问时是否复用上一轮 `response_id`。

## 功能配置

- `/translate`、`/fy`、`/tr` 读取功能页里的翻译提示词和翻译 LLM 模型。
- 翻译模型是 `responses` 就走 `/responses`，是 `chat/completions` 就走 `/chat/completions`。
- 翻译只使用功能页显式选择的翻译 LLM。
- 内置默认翻译提示词把英文和简体中文视为核心语言对；未显式指定目标语言时按“简中 -> 英文、英文 -> 简中、其他语言 -> 简中”处理，并要求保留原文语气、风格和格式，只返回译文。
- 文档问答的回答阶段读取功能页里的系统提示词和问答 LLM 模型；检索阶段仍由知识库页配置 Embedding。
- 保存翻译配置不能把文档问答草稿一起写回；保存文档问答配置也不能覆盖翻译草稿。

## OCR

- OCR provider 支持 `system`、`llm_ocr`、`disabled`。
- `llm_ocr` 不直接保存 URL / API key，而是引用某个已配置的 LLM 模型。
- OCR 保存时要求所选 LLM 条目存在、类型是普通 LLM、协议是 `responses`，且 `supportsMultimodal = true`；否则后端拒绝落盘。
- 远程 OCR 当前调用所选条目的 OpenAI 兼容 `responses` 接口，把截图编码成 data URL 作为多模态输入。
- 截图采集 backend 是 macOS ScreenCaptureKit；设置页不提供尚未实现的高级 OCR、vision prompt 或屏幕解析配置。
- 无选中文本的 `Alt+D` 翻译路径在 OCR disabled 时应给出明确不可用提示。

## 知识库

- 知识库只接受 Embedding 类型条目。
- 没有选 provider 时允许保存空白配置；只要配置了扫描目录，就必须同时配置 Embedding 条目。
- 保存设置不会默认触发全量重建；只有改变有效索引结果的输入发生变化时，后端才会自动重建索引：当前 Embedding 条目、扫描目录或忽略规则变化都会命中。
- 其它场景需要用户显式点击“立即重建索引”。
- 扫描目录使用“每行一个目录”，保存后 watcher 只监听这些显式选中的目录。
- 支持 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc`、`.docx`、`.pdf`。
- 扫描边界只受“显式选择的目录 + 生效中的 ignore glob + 50 MB 大小上限”控制。
- `.gitignore` / `.ignore` / 全局 git ignore 不会被当成额外隐式过滤条件。
- 纯文本类文件要求内容可读、是 UTF-8 文本且不含 NUL 字节。
- `.md`、`.mdx`、`.markdown`、`.docx` 会按标题、列表项、代码块和普通段落做语义预切，再按字符预算打包。
- `.txt`、`.rst`、`.adoc` 走通用文本切分。
- 文本型 `.pdf` 按页抽取文本；允许页内 chunk 部分失败并保留可读片段。
- 明显控制字符污染或可疑乱码页会被质量闸门丢弃。
- 忽略规则分为固定内置目录和额外 ignore glob。内置规则包含常见第三方依赖目录和编译产物目录，例如 `node_modules`、`target`、`dist`、`.next`、`.git`、`coverage`，始终生效且不支持取消。

## Agent 和 MCP

- 翻译 LLM 和问答 LLM 直接服务 launcher 的翻译 / 文档问答链路。
- Agent session 会优先复用功能页的文档问答模型。
- 只有当该模型所属 provider 能安全映射到 Pi SDK 已知 provider（OpenAI / OpenRouter / DeepSeek / Ollama / SiliconFlow）时才自动传入 SDK。
- 其它 provider 或缺失问答模型时，Agent 回退到 Pi SDK 自身配置。
- Agent 面板只展示运行时说明，不提供独立保存动作。
- Wabity 不再维护多个外部 agent 命令目录、启动模式或安装预设。
- 全局 MCP 清单当前不自动注入 Agent session；后续若要注入 Pi session，必须通过 Pi SDK `ToolFactory` 明确建模。
- MCP 服务支持 `stdio`、`http`、`sse`。
- `stdio` 使用“每行一个参数 / KEY=VALUE env”。
- `http` / `sse` 使用“服务地址 + 每行一个 KEY=VALUE header”。
- 远程 URL 必须是完整的 `http://` / `https://` 地址。
- 内置 MCP 不伪装成普通服务草稿；保存时同时写回自定义服务清单和内置 MCP 模块配置。
