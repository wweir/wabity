# domain

职责：

- 定义 launcher 的输入模型、动作模型、执行请求与结果模型
- 定义设置页通用/外观/提示词/LLM/OCR/RAG 配置模型
- 定义 workspace、Agent session 等前后端通信结构
- 定义历史剪贴板快照和条目模型
- 定义文件搜索结果等前后端通信结构
- 定义已安装应用搜索结果等前后端通信结构
- 保持 Rust 与前端通信的结构化边界稳定

原则：

- 输入模式必须显式分型
- 动作能力必须声明支持的输入模式
- 动作元数据必须提供面向用户的简短说明，前端候选列表不展示内部评分细节
- 执行结果必须包含状态、文本结果和结构化副作用 hint
- workspace 状态必须显式包含当前根目录、最近目录，以及前端安全做 `HOME -> ~` 缩写所需的上下文
- 历史剪贴板模型必须显式区分 `pinned` 条目和 `recent` 条目；同一段文本只保留一份记录，不允许前后端各自再拼分组规则
- Agent session 状态必须显式区分摘要信息和消息流详情；摘要必须能区分可恢复错误和不可恢复错误
- 设置模型必须显式表达通用、外观、提示词、LLM 配置目录、OCR 配置和 RAG 配置，避免前端把持久化字段散落成匿名对象；翻译提示词和 RAG 问答系统提示词必须落在独立 `PromptsSettings`
- LLM 配置模型必须显式分成两层：provider 层保存 `base_url`、`api_key`、`protocol`，model 层保存 `id`、`model_type`、`model`、`model_identity_hint`、`builtin_preset_model_id`、`supports_multimodal` 和 `supports_stateful`；provider 是分组容器，一个 provider 下可以维护多个模型，真正被翻译 / OCR / RAG 引用的是具体 `modelId`
- `LlmProviderConfig` 保存的是用户输入与模板绑定，不直接充当运行时用途资格；像“是否可供翻译 / 问答 / OCR / Embedding 复用”“当前走哪条 OpenAI-compatible 协议”“是否允许 stateful / multimodal”这类语义，必须先投影成统一的 provider profile，再供校验、设置页说明和运行时服务复用。这里要显式区分两类能力：普通 LLM 的多模态推理能力，以及 `Embedding` 条目是否接受结构化多模态 input；后者不自动意味着可供 OCR / 问答使用
- 内置 LLM 供应商模板目录也必须显式建模；模板目录属于只读发布物，用户最终保存的仍然是普通 `LlmProviderConfig` 条目。条目若来自内置模板，还要显式携带模板来源字段，避免前端靠字符串猜来源
- 翻译能力依赖显式选择的普通 LLM 模型；Embedding 模型不能被误用成通用文本模型
- 问答请求模型必须显式携带多轮历史和续链状态；`ExecutionRequest.conversation` 只表达 launcher 问答内部的 user/assistant turn，`ExecutionRequest.conversation_state` 除了 `previous_response_id` 外，还要显式携带 `continuation_scope`、累计 citation、累计工具摘要和 action 轨迹。后端只会在 scope 与“当前 provider + 当前 workspace”一致时复用这些状态，不复用 Agent session 结构，也不允许跨 provider/workspace 误续链
- RAG 模型必须显式表达扫描目录、忽略 glob、embedding 条目引用和一次扫描返回的统计结果，避免前后端各自拼临时结构
- 内置 RAG MCP server 的状态也必须显式建模，避免前端把“固定 URL + 运行状态 + 最近错误”拼成匿名对象
- RAG 问答结果必须稳定表达引用信息；当前继续复用 `ExecutionResult.structured_payload` 的弱类型 JSON，但已经固定 `kind`、`render`、`responseId`、`reasoning`、`citations`、`retrieval`、`actions` 这些字段，且 citation 必须显式带 `path`、`absolutePath`、`documentKind`、`lineStart`、`lineEnd`、`paragraphLineStart`、`pageStart`、`pageEnd`、`headingPath`、`anchorLabel`，不能让前后端各自猜 payload 形状；文本文件保留行号强语义，PDF 这类抽取型文档允许行号为空并改用页码锚点；`reasoning` 只表达次级思考内容，不能与主答案混淆；`actions` 复用 Agent action event 形状承载问答中的多步操作、tool call 输入和 tool result 输出，工具摘要额外挂在同一个 payload 上，前端可忽略但不能破坏兼容
- 翻译结果也允许通过 `ExecutionResult.structured_payload` 携带次级信息；当前固定 `kind = translation_result` 和可选 `reasoning` 字段。只有在 provider 同时给出明确译文时，这段 `reasoning` 才允许下发给前端；若只有 thinking 没有正文，服务必须报错，不能再让前端猜测哪个字段算译文
- Agent 第一阶段优先复用 Wabity 已配置的文档问答 LLM；若该模型所属 provider 可映射到 Pi SDK 已知 provider（OpenAI / OpenRouter / DeepSeek / Ollama / SiliconFlow），后端会把 provider、model 和 api_key 传入 `SessionOptions`。无法安全映射的 provider 继续回退到 Pi SDK 自身配置。Wabity 领域模型仍不表达外部 agent 命令、启动模式或默认 ACP agent 目录
- 全局 MCP 配置必须能稳定表达 MCP server 清单，至少覆盖 `stdio/http/sse` 三类 transport，以及 `args`、`env`、`headers` 这些连接参数
- 文件搜索结果必须返回稳定路径、文件名和父目录信息
- 应用搜索结果至少返回展示名、bundle 路径和排序分数；服务内部的索引记录可以携带展示名和检索别名，但不能把“应用对象”伪装成普通 action
