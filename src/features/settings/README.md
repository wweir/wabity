# settings feature

职责：

- 渲染设置页 UI
- 展示并修改通用设置、外观设置、快捷键、多 LLM provider 配置、OCR provider、RAG 配置、ACP agent 配置和全局 MCP 清单
- `settings.css` 只保留设置页特有布局、表单状态和卡片变体；基础 token、frame/panel/button/input 外观统一由 `src/app/global.css` 提供
- 设置页默认优先使用输入框、下拉框、开关等原生表单控件；没有明显的信息分组或选择密度收益时，不引入额外卡片交互
- 通过一级导航把通用、快捷键、外观、LLM、RAG、ACP Agent、MCP、Skills、关于拆成独立设置分组，而不是单个长表单；OCR 配置并入通用页
- ACP Agent 和 MCP 页面都改成表单优先：已配置列表放在表单上方，页面主体直接围绕当前表单编辑
- Skills 页面只读展示 `~/.agents/skills` 下的公共 skill：左侧列出 skill，右侧查看 `SKILL.md` meta、目录/文件统计和目录树
- 对 ACP agent 名称、启动命令、MCP 必填字段和 `KEY=VALUE` 文本格式做前端即时校验
- 将通用/外观配置实时持久化到后端 `config.toml`
- 将 LLM provider 目录、默认 provider、OCR provider、OCR 引用的 LLM provider、RAG 扫描目录、忽略 glob 和 embedding provider 写回后端配置
- LLM 页面允许维护多个 provider 条目；每个条目显式保存 Base URL、API key、model、协议 `openai_chat` / `openai_responses` / `openai_embedding` 以及 `supportsMultimodal`
- LLM、OCR、RAG 页面各自维护草稿；保存某个分组时不会把其他分组的未保存草稿偷偷带进 `config.toml`
- RAG 页面允许选择 embedding provider、维护多个扫描目录、配置忽略 glob，并手动触发一次全量重建；保存后后端会按配置重启目录监听
- RAG 页面会明确展示当前允许向量化的文本后缀，并把扫描目录示例收敛为 `~/Documents`，不再写死开发机路径
- 将快捷键、ACP agent 配置和全局 MCP 配置通过独立命令写回后端
- 页面初始化时从后端读取当前配置，而不是依赖前端默认值假装“已保存”
- 保存失败时保持旧值，不把前端草稿伪装成已落盘状态
- 维持固定面板高度策略：设置页窗口高度按当前显示器可用高度做上限裁剪，表单内容始终在页内单滚动区滚动

接口：

- `getAppSettings` / `setAppSettings`：读取和写入通用/外观/LLM/OCR/RAG 配置
- `scanRagSources`：用当前草稿立即触发一次 RAG 全量扫描，返回 LanceDB 路径和统计结果
- `getShortcut` / `setShortcut`：读取和更新快捷键
- `getAcpAgents` / `setAcpAgents`：读取和更新 ACP agent 列表；接口仍透出 `defaultAgentId`，但前端只把它当兼容/兜底字段保存，设置页不提供修改入口
- `getAcpMcpServers` / `setAcpMcpServers`：读取和更新全局 MCP server 清单
- `getPublicSkillCatalog`：只读扫描公共 skill 目录并返回 skill meta、目录/文件统计和目录树

约束：

- 设置页本身不解释配置文件路径或格式，持久化细节由 Rust `ConfigStore` 负责
- 配置文件路径由后端固定为 `dirs::config_dir()/wabity/config.toml`
- 写入由后端 `safe_write` 原子替换，前端只有在命令成功后才提交新状态
- 默认快捷键当前为 launcher=`Cmd+Shift+Space`、截图 OCR=`Cmd+Shift+O`；非 macOS 分别退化为 `Ctrl+Shift+Space`、`Ctrl+Shift+O`
- 后端加载配置时会把历史默认 OCR 快捷键自动迁移到新默认值，但不会覆盖用户自定义快捷键
- LLM provider 当前支持多条目录项；`Base URL` 由用户显式填写到 provider API 根路径，通常包含 `/v1`，`API key` 字段会在模型选择前展示，便于先完成连接信息
- LLM provider 的 `API key` 允许为空，兼容本地或内网 OpenAI 兼容网关；`model` 必填。设置页会根据当前 `baseUrl + apiKey` 调 `/models` 拉模型列表，先提供明确下拉选项，再保留一个可手填的输入框，允许填写列表里没有的模型
- 协议选择使用下拉框，不用卡片；它和 `supportsMultimodal` 放在同一行，避免无意义的纵向拉长
- 切换协议不会清空已拉取的模型目录，但 `openai_embedding` 仍会强制关闭多模态
- `supportsMultimodal` 只对 `openai_chat` / `openai_responses` 有意义；`openai_embedding` 会被强制视为非多模态
- OCR provider 当前支持 `system`、`llm_ocr` 和 `disabled`；`llm_ocr` 不再直接保存 URL/API key，而是引用某个已配置的 LLM provider
- OCR 配置放在通用页内的 editor card；只有 provider 选中 `llm_ocr` 时才显示 LLM provider 选择器，切走其他 provider 时仅隐藏，不主动清空草稿
- OCR 保存时要求所选 LLM provider 存在、协议为 `openai_chat` 且 `supportsMultimodal = true`；否则后端拒绝落盘
- 远程 OCR 当前调用所选 provider 的 OpenAI 兼容 `chat/completions` 接口，把截图编码成 data URL 作为多模态输入；截图采集链路仍然只在 macOS 下可用
- RAG 只接受协议为 `openai_embedding` 的 provider；没有选 provider 时允许保存空白配置，但只要配置了扫描目录就必须同时配置 embedding provider
- RAG 扫描目录使用“每行一个目录”的 textarea，并提供“选择目录追加”按钮；保存后 watcher 只监听这些显式选中的目录
- RAG 当前只向量化后缀为 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc` 的文件；除此之外还要求文件可读、是 UTF-8 文本、大小不超过 50 MB，且不命中忽略 glob
- `.md`、`.mdx`、`.markdown` 会按文档结构切分，其余文本后缀走通用语义切分
- RAG 忽略规则使用“每行一个 glob”的 textarea；命中后文件不会被切分、向量化或写入 LanceDB
- 默认 LLM provider 只是设置页和 OCR 的兜底引用，不直接决定 ACP session 该用哪个 agent
- 可以同时配置多个 ACP agent，但每个 session 仍然只绑定其中一个
- 所有 ACP agent 共用同一份全局 MCP 清单；设置页不再支持按 agent 分别挂 MCP
- Skills 页是只读浏览器，不允许从设置页直接修改 `~/.agents/skills` 内容
- MCP server 当前支持 `stdio/http/sse` 三类 transport；`stdio` 用“每行一个参数 / KEY=VALUE env”，`http/sse` 用“URL + 每行一个 KEY=VALUE header”编辑
- ACP Agent 页面以表单为主：顶部不再放独立概览卡，而是在已配置列表上方用一句话解释 ACP Agent；预设下拉和编辑字段在其后
- MCP 页面单独负责维护全局 server 清单；连接类型通过和 ACP 预设相同的“双栏卡片”创建空白表单，左侧选择 transport，右侧说明当前 transport 会创建哪些字段和示例，保存动作与 Agent 保存彼此独立
- ACP Agent 页面里的预设只用于填充名称和启动命令这类表单默认值，不负责决定默认 agent；真正创建 session 用哪个 agent，由 launcher 顶部 Agent 菜单决定
- ACP Agent 页面在“选择 Agent”卡片右侧补一条极简安装提示，并让右侧安装提示显著宽于左侧选择区；窄屏时再回落成上下堆叠
- ACP Agent 安装卡片除了安装命令，还要提供一句基础介绍和官方链接，避免用户只看到命令却不知道这个预设实际接的是什么 agent
- ACP Agent 预设里的安装提示需要跟随各 agent 官方安装文档更新，不能保留未证实的第三方安装方式；对 Codex 这类官方同时提供 `npx` 试运行和长期安装路径的预设，要明确区分“临时运行”和“装到 PATH”
- “选择 Agent” 下拉项直接展示 agent 名称、对应启动命令和已配置状态，减少试错
- “选择 Agent” 下拉框移除空白占位项，首项固定为“自定义 · 空白表单”
- “选择 Agent” 左侧改成完整输入模块：下拉框在上、填入按钮在下，并补一行只说明“填默认值、不直接保存”的辅助文案
- 预设配置改成下拉选择项；重复添加预设会给出“已存在”提示并跳到对应 agent，而不是静默无响应
- ACP 详情面板底部使用粘底保存栏；存在校验错误时优先提供“定位问题”动作，直接跳到第一个非法字段
- 设置页不再跟随表单内容无限抬高窗口；窗口高度只在进入设置页时按显示器可用高度收敛一次，后续由内容区滚动承接溢出
