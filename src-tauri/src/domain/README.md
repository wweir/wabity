# domain

职责：

- 定义 launcher 的输入模型、动作模型、执行请求与结果模型
- 定义设置页通用/外观/LLM/OCR/RAG 配置模型
- 定义公共 skill 目录浏览模型
- 定义 workspace、ACP session 等前后端通信结构
- 定义文件搜索结果等前后端通信结构
- 定义已安装应用搜索结果等前后端通信结构
- 保持 Rust 与前端通信的结构化边界稳定

原则：

- 输入模式必须显式分型
- 动作能力必须声明支持的输入模式
- 动作元数据必须提供面向用户的简短说明，前端候选列表不展示内部评分细节
- 执行结果必须包含状态、文本结果和结构化副作用 hint
- workspace 状态必须显式包含当前根目录、最近目录，以及前端安全做 `HOME -> ~` 缩写所需的上下文
- ACP session 状态必须显式区分摘要信息和消息流详情；摘要必须能区分可恢复错误和不可恢复错误
- 设置模型必须显式表达通用、外观、LLM provider 目录、OCR 配置和 RAG 配置，避免前端把持久化字段散落成匿名对象
- LLM provider 模型必须显式表达协议类型、`base_url`、`api_key`、`model` 和多模态声明；协议当前收敛为 `openai_chat` / `openai_responses` / `openai_embedding`，避免 OCR 或后续功能依赖隐式约定
- RAG 模型必须显式表达扫描目录、忽略 glob、embedding provider 引用和一次扫描返回的统计结果，避免前后端各自拼临时结构
- 公共 skill 浏览模型必须把 meta、统计和目录树拆成显式字段，避免前端重新解析 `SKILL.md`
- ACP agent 配置必须能表达直接启动和 shell 启动两种模式，并支持“多个已配置 agent + 一个默认 agent”的目录式管理
- 全局 MCP 配置必须能稳定表达 MCP server 清单，至少覆盖 `stdio/http/sse` 三类 transport，以及 `args`、`env`、`headers` 这些连接参数
- 文件搜索结果必须返回稳定路径、文件名和父目录信息
- 应用搜索结果至少返回展示名、bundle 路径和排序分数；服务内部的索引记录可以携带展示名和检索别名，但不能把“应用对象”伪装成普通 action
