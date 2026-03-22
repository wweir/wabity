# settings feature

职责：

- 渲染设置页 UI
- 展示并修改通用设置、AI 功能配置、多 LLM 模型条目配置、OCR provider、RAG 配置、ACP agent 配置和全局 MCP 清单
- `settings.css` 只保留设置页特有布局、表单状态和卡片变体；基础 token、frame/panel/button/input 外观统一由 `src/app/global.css` 提供
- 透明窗口策略下，设置页只保留极小透明安全边；frame 背景和 overlay 必须跟随全局 surface token，不能继续写死浅色 frosted glass，否则暗色主题会失真
- 设置页内部的编辑卡片、空态、安装指引、内联代码和按钮同样必须跟随全局 surface / text token；不能只修外层 frame，留下暗色主题里的浅底内卡和深色文字
- LLM 页面里的模型名下拉浮层、展开/刷新按钮和“多模态”能力卡也属于同一套 surface / border token 管辖范围；暗色主题下禁止残留浅底能力卡或亮色浮层
- 设置页和 launcher 共用一套冷静、专业、平和的中性色外观语言；frame、导航、表单和卡片优先复用全局 token，不再继续堆 frosted glass 风格
- 所有自定义按钮、导航项和 jump item 都先清掉原生 `appearance`，再应用 token；否则深色主题下会重新冒出浅色系统按钮皮肤
- 设置页的 focus ring、active/hover/disabled 状态也必须使用语义 token；不要再靠白色 inset 高光、统一降透明度或遗留浅色常量勉强适配 dark 模式
- 设置页默认优先使用输入框、下拉框、开关等原生表单控件；没有明显的信息分组或选择密度收益时，不引入额外卡片交互
- 通过一级导航把通用、AI 功能、LLM、RAG、ACP Agent、MCP、Skill、关于拆成独立设置分组，而不是单个长表单；快捷键、外观和 OCR 配置并入通用页
- 一级导航按真实 `tablist/tab/tabpanel` 语义实现，并支持键盘方向键、`Home`、`End` 切换，避免只剩视觉高亮没有状态语义
- 设置页整体交互改成左侧常驻导航：左侧只负责分组切换，当前分组的块级 jump rail 固定在主内容区标题下方，避免在窄侧栏里堆叠过多导航层级
- 设置页的滚动容器固定在右侧主内容区；左侧分组导航不跟着右侧配置滚动，避免长表单场景里导航位置漂移
- 左侧导航列保持窄宽度，优先把可用宽度让给右侧编辑区；右侧配置卡、说明卡和长命令文本必须允许收缩换行，不能让设置页出现横向滚动条
- 主内容区顶部提供紧凑 quick jump：在窄窗口下继续保留块级跳转，但不再用侵占式 sticky 浮条反复压缩可视空间
- 左侧导航、主内容 jump rail 和 section toolbar 只保留一层必要容器；重复说明块、重复跳转文案和模板化包裹层要删掉，避免 settings 在暗色主题里显得碎和挤
- 右侧 quick jump 和 section toolbar 默认使用普通流内布局，不做长期悬浮；窗口本身较小时，sticky 浮块会持续侵占可视空间
- 需要显式保存的分组统一把保存、恢复已保存版本、定位问题动作收敛到主编辑区内的草稿操作卡；分组标题和窗口头部都不再承载保存语义
- 快捷键录制改成显式按钮 + 状态文案，而不是伪装成只读输入框；必须支持键盘触发和录制态反馈
- ACP Agent 和 MCP 页面都改成表单优先：MCP 的已配置服务目录改成顶部两列卡片；主区只在“当前服务”和“新建服务”两种模式里二选一，避免同时展示两个互相竞争的主任务
- Skills 页面只读展示 `~/.agents/skills` 下的公共 skill：顶部用每排 3 个的小卡片列出目录，下面查看 `SKILL.md` meta、目录/文件统计和目录树
- 对 ACP agent 名称、启动命令、MCP 必填字段和 `KEY=VALUE` 文本格式做前端即时校验；MCP 远程 URL 在前后端都必须限制为完整的 `http://` / `https://` 地址。字段级错误必须直接绑定到对应控件，通过 `aria-invalid` / `aria-describedby` 暴露给辅助技术，而不是只在分组底部堆总错误列表
- 将通用/外观配置实时持久化到后端 `config.toml`
- 通用页里的 `autoStart` 不只是写 `config.toml`；Rust 侧保存成功后必须同步系统登录启动项，应用启动时还会再做一次配置与系统状态对账
- 外观设置除了持久化，还必须立即同步到运行中的前端根节点：`theme` 负责 `data-theme` / `color-scheme`，`fontSize` 负责 `data-font-size`
- 将 AI 功能配置以独立分组写回后端 `config.toml`，覆盖翻译提示词、RAG 问答系统提示词、翻译 LLM 和问答 LLM 选择
- 将 LLM 条目目录、OCR provider、OCR 引用的 LLM 条目、RAG 扫描目录、忽略 glob 和 Embedding 条目写回后端配置
- LLM 页面允许维护多个条目；每个条目显式保存 Base URL、API key、`modelType`、`protocol`、`model`、`supportsMultimodal` 和 `supportsStateful`
- LLM 页面条目选择区使用卡片网格，而不是普通列表；每张卡片直接展示用途标签、Base URL、合并后的“配置类型 + 模型名”标签和校验问题数，并按类型给 `LLM` / `Embedding` 不同背景色，减少多条目切换时的扫读成本
- LLM 条目卡片只负责展示条目本身和当前用途标签；翻译 / 问答模型选择统一移到 AI 功能页，LLM 页不再承载“默认 LLM”逻辑
- 单个 LLM 条目的编辑区改成“先选配置类型，再填基础信息和模型名”；配置类型把 `modelType + protocol + supportsStateful` 合并成单个选择器，直接区分 `LLM · responses stateless`、`LLM · responses stateful`、`LLM · chat/completions` 和 `Embedding` 四种页面。用途说明收敛到单个摘要区：`responses` 页面仍可配置多模态，`chat/completions` 页面可进入翻译和 RAG 问答但不进入 OCR；Embedding 类型只在必要时提示 RAG 重建影响
- LLM、OCR、RAG 页面各自维护草稿；保存某个分组时不会把其他分组的未保存草稿偷偷带进 `config.toml`
- RAG 页面使用和 LLM 页一致的双栏信息架构：左侧摘要当前 Embedding、目录数、忽略规则数和支持后缀，右侧用 hero、主编辑器和辅助说明卡分别承载索引流程、配置输入、数据边界和最近一次手动重建结果
- RAG 页面允许选择 Embedding 条目、维护多个扫描目录、配置忽略 glob，并手动触发一次全量重建；保存后后端会按配置重启目录监听
- RAG 页面会提醒“Embedding 模型变化会触发向量重建”；LLM 页面编辑被 RAG 引用的 Embedding 条目时，也会明确提示保存后会重建相关向量
- RAG 页面会明确展示当前允许向量化的文本后缀，并把扫描目录示例收敛为 `~/Documents`，不再写死开发机路径
- 将快捷键、ACP agent 配置和全局 MCP 配置通过独立命令写回后端
- 页面初始化时从后端读取当前配置，而不是依赖前端默认值假装“已保存”
- 保存失败时保持旧值，不把前端草稿伪装成已落盘状态
- 维持固定面板尺寸策略：设置页窗口宽高都按当前显示器可用区域做上限裁剪，默认目标规格为 `920 x 720`；表单内容始终在页内单滚动区滚动
- 窄窗口优先退让布局而不是继续压缩内容：侧栏导航可折成单列或双列，主内容区 sticky quick jump 保留，主操作按钮和表单触达尺寸维持 `44px`；浏览器预览和非桌面壳场景下的 settings frame 宽度必须优先跟随 `window.innerWidth`，不能被 `screen.availWidth` 或共享最小宽度错误钉死

接口：

- `getAppSettings` / `setAppSettings`：读取和写入通用/外观/AI 功能/LLM/OCR/RAG 配置
- `scanRagSources`：用当前草稿立即触发一次 RAG 全量扫描，返回 LanceDB 路径和统计结果
- `getShortcut` / `setShortcut`：读取和更新快捷键
- `getAcpAgents` / `setAcpAgents`：读取和更新 ACP agent 列表；接口仍透出 `defaultAgentId`，但前端只把它当兼容/兜底字段保存，设置页不提供修改入口
- `getAcpMcpServers` / `setAcpMcpServers`：读取和更新全局 MCP server 清单
- `getPublicSkillCatalog`：只读扫描公共 skill 目录并返回 skill meta、目录/文件统计和目录树

约束：

- 设置页本身不解释配置文件路径或格式，持久化细节由 Rust `ConfigStore` 负责
- 配置文件路径由后端固定为 `dirs::config_dir()/wabity/config.toml`
- 写入由后端 `safe_write` 原子替换，前端只有在命令成功后才提交新状态
- 保存设置不会默认触发 RAG 全量重建；只有会改变有效索引结果的输入发生变化时，后端才会自动重建索引：当前 Embedding 条目、扫描目录或忽略规则变化都会命中这一条件。其它场景需要用户显式点击“立即重建索引”
- 默认快捷键当前为 launcher=`Alt+Space`、截图 OCR=`Alt+R`、优先翻译选中文本否则截图 OCR 并翻译=`Alt+D`
- 后端加载配置时会把历史默认 OCR 快捷键自动迁移到新默认值，但不会覆盖用户自定义快捷键
- LLM 条目当前支持多条目录项；`Base URL` 由用户显式填写到 API 根路径，通常包含 `/v1`，`API key` 字段会在模型选择前展示，便于先完成连接信息
- LLM 条目的 `API key` 允许为空，兼容本地或内网 OpenAI 兼容网关；每个条目必须先选“配置类型”，再填写唯一的 `model`。设置页会根据当前 `baseUrl + apiKey` 调 `/models` 拉模型列表；模型输入区收敛为“可手填输入框 + 右侧下拉按钮 + 浮层列表”，用户既可以直接从列表回填，也可以手动填写列表里没有的模型
- `supportsMultimodal` 当前只对 `responses` 页面有意义；切到 `chat/completions` 或 Embedding 页面时会固定关闭
- `supportsStateful` 不再通过独立开关暴露，而是直接折叠进“配置类型”选择器；它仍只控制 launcher 文档问答继续追问时是否复用上一轮 `response_id`
- OCR provider 当前支持 `system`、`llm_ocr` 和 `disabled`；`llm_ocr` 不再直接保存 URL/API key，而是引用某个已配置的 LLM 条目
- OCR 配置放在通用页内的 editor card；只有 provider 选中 `llm_ocr` 时才显示 LLM 条目选择器，切走其他 provider 时仅隐藏，不主动清空草稿
- AI 功能页按“翻译配置 / 文档问答配置”两个独立任务卡组织；每张卡同时编辑该任务使用的 LLM 条目和系统提示词，并各自提供“恢复默认 / 恢复已保存版本 / 保存”动作。保存翻译配置时不能把文档问答草稿一起写回，保存文档问答配置时也不能覆盖翻译草稿
- `/translate`、`/fy`、`/tr` 会读取 AI 功能页里的翻译提示词和翻译 LLM；所选条目是 `responses` 就走 `/responses`，是 `chat/completions` 就走 `/chat/completions`
- RAG 问答的回答阶段会读取 AI 功能页里的系统提示词和问答 LLM；检索阶段仍然由 RAG 页面配置 Embedding 条目
- OCR 保存时要求所选 LLM 条目存在、类型是普通 LLM、协议是 `responses`，且 `supportsMultimodal = true`；否则后端拒绝落盘
- 翻译只使用 AI 功能页中显式选择的翻译 LLM；当前同时支持 `responses` 和 `chat/completions` 两种协议。内置默认提示词把英文和简体中文视为核心语言对；未显式指定目标语言时按“简中->英文、英文->简中、其他语言->简中”处理，并要求保留原文语气、风格和格式，只返回译文
- 远程 OCR 当前调用所选条目的 OpenAI 兼容 `responses` 接口，把截图编码成 data URL 作为多模态输入；截图采集链路仍然只在 macOS 下可用
- RAG 只接受 Embedding 类型条目；没有选 provider 时允许保存空白配置，但只要配置了扫描目录就必须同时配置 Embedding 条目
- RAG 扫描目录使用“每行一个目录”的 textarea，并提供“选择目录追加”按钮；保存后 watcher 只监听这些显式选中的目录
- RAG 当前只向量化后缀为 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc` 的文件；除此之外还要求文件可读、是 UTF-8 文本、大小不超过 50 MB，且不命中忽略 glob。`.gitignore` / `.ignore` / 全局 git ignore 不会被当成额外隐式过滤条件
- `.md`、`.mdx`、`.markdown` 会按文档结构切分，其余文本后缀走通用语义切分
- RAG 忽略规则使用“每行一个 glob”的 textarea；默认草稿会预填常见第三方依赖目录和编译产物目录，例如 `node_modules`、`target`、`dist`、`.next`，命中后文件不会被切分、向量化或写入 LanceDB
- 翻译 LLM 和问答 LLM 只影响 launcher 的翻译 / RAG 问答链路，不直接决定 ACP session 该用哪个 agent
- 可以同时配置多个 ACP agent，但每个 session 仍然只绑定其中一个
- 所有 ACP agent 共用同一份全局 MCP 清单；设置页不再支持按 agent 分别挂 MCP
- Skills 页是只读浏览器，不允许从设置页直接修改 `~/.agents/skills` 内容
- MCP server 当前支持 `stdio/http/sse` 三类 transport；`stdio` 用“每行一个参数 / KEY=VALUE env”，`http/sse` 用“URL + 每行一个 KEY=VALUE header”编辑；远程 URL 必须是完整的 `http://` / `https://` 地址
- ACP Agent 页面以表单为主：顶部不再放独立概览卡，而是在已配置列表上方用一句话解释 ACP Agent；预设下拉和编辑字段在其后
- MCP 页面单独负责维护全局 server 清单；transport 只在创建时决定，编辑态明确提示“需要切换 transport 就新建一个服务”；新增入口退到当前表单后面的辅助卡，不再作为首屏主内容；创建新服务后会自动切回当前表单并聚焦到第一个可编辑字段
- 内置 `Wabity RAG Query` 不再塞进“新建服务”表单；目录区固定展示一张内置 MCP 卡片，用运行状态 + 开关控制是否把它写入当前 MCP 草稿，避免把内置入口和空白创建流程混在一起
- ACP Agent 页面里的预设只用于填充名称和启动命令这类表单默认值，不负责决定默认 agent；真正创建 session 用哪个 agent，由 launcher 顶部 Agent 菜单决定
- ACP Agent 页面在“选择 Agent”卡片右侧补一条极简安装提示，并让右侧安装提示显著宽于左侧选择区；窄屏时再回落成上下堆叠
- ACP Agent 安装卡片除了安装命令，还要提供一句基础介绍和官方链接，避免用户只看到命令却不知道这个预设实际接的是什么 agent
- ACP Agent 预设里的安装提示需要跟随各 agent 官方安装文档更新，不能保留未证实的第三方安装方式；对 Codex 这类官方同时提供 `npx` 试运行和长期安装路径的预设，要明确区分“临时运行”和“装到 PATH”
- “选择 Agent” 下拉项直接展示 agent 名称、对应启动命令和已配置状态，减少试错
- “选择 Agent” 下拉框移除空白占位项，首项固定为“自定义 · 空白表单”
- “选择 Agent” 左侧改成完整输入模块：下拉框在上、填入按钮在下，并补一行只说明“填默认值、不直接保存”的辅助文案
- 预设配置改成下拉选择项；重复添加预设会给出“已存在”提示并跳到对应 agent，而不是静默无响应
- ACP Agent 和 MCP 分组的保存动作统一收敛到主编辑区内的草稿操作卡；提交时如有校验错误，直接定位到第一个非法字段
- 设置页不再跟随表单内容无限抬高或拉宽窗口；窗口宽高只在进入设置页时按显示器可用区域收敛一次，后续由内容区滚动承接溢出
- 非激活分组不再继续常驻挂载在 DOM 里；切换分组时只渲染当前 `tabpanel`，降低设置页整页重渲染成本
