# RAG QA Design

## 实现状态

当前仓库已经落地 v1，且正在演进到 v2：

- 后端新增 `rag_answer` 服务，并由 `AppState::execute_action` 专门调度
- launcher 新增显式 slash 动作 `/ask` / `/qa` / `/docs`
- 普通文本模式下，如果应用搜索没有弹出补全框，主动作默认回退到问答
- 问答答案走 Markdown 渲染，引用通过独立 `References` 列表展示
- 点击引用会通过 `open_document_reference(path)` 打开本地文件
- v2 起，问答不再自动预注入 RAG 命中片段，而是给模型注入内置 `wabity.rag.query` / `wabity.read_file_lines` 和全局 HTTP/SSE MCP server，由模型自己发起工具调用；其中 `wabity.read_file_lines` 只允许读取当前 workspace 和显式配置的 RAG source roots
- v2 起，launcher 会把最近几轮问答的 user/assistant 文本显式回传给后端，形成轻量多轮上下文；这仍然不是 ACP session
- `Esc` 显式隐藏 launcher 时会把这份轻量多轮上下文连同当前输入、内联结果和当前激活 session 选择一起清掉，回到干净的 launcher 初始态；其它隐藏路径只隐藏窗口，继续保留上下文
- v2 起，问答请求会显式打开 `parallel_tool_calls`，并在模型返回多个本地 function call 时并发执行，再把 tool output 回填给下一轮 `responses`
- v2.1 起，问答同时支持 `chat/completions`、`responses stateless` 和 `responses stateful`；只有 `responses` 会继续注入 HTTP/SSE MCP server
- v2.2 起，问答后端新增独立模块 `question_answer_backend` 作为稳定函数入口；`AppState` 只负责装配依赖，`src-tauri/tests/` 可直接用该模块做集成测试

## 目标

基于现有 RAG 向量索引和 MCP 配置，补一条“从 launcher 直接提问”的问答链路：

- 用户在 launcher 输入问题
- 系统把读文件、RAG Query 和可直连的 MCP server 作为工具注入给模型
- 模型按需自行检索、读文件、调用外部 MCP 工具
- 工具结果回填后再生成答案
- launcher 内联显示答案
- 并保留轻量多轮历史，支持继续追问

这个能力现在是“轻量多轮问答”，但仍然不是 ACP session 替代品。不要把它设计成另一套长期对话系统，也不要把 ACP 的会话消息模型强塞到 launcher 问答里。

## 先把问题说清楚

当前仓库已经有：

- RAG 建索引和 watcher 增量维护
- launcher slash 动作匹配与执行链路
- OpenAI 兼容 LLM 调用
- Markdown 结果渲染

当前仓库在本设计落地前还没有：

- 向量检索查询链路
- 面向 RAG 问答的动作
- 文档引用点击打开能力

更关键的约束是：

- launcher 当前普通文本入口首先服务应用搜索
- 非 `http(s)` Markdown 链接现在只会显示成静态文本，不会打开本地文件

所以“直接提问”这个说法如果翻译成“任意普通文本都直接走问答”，就是错误设计。  
正确的落点应是：普通文本先保留应用搜索机会，只有应用搜索没有弹出补全框时，默认主动作才回退到 RAG 问答。

## 推荐方案

### 1. 交互入口

推荐做成双入口：

- 主命令：`/ask`
- 别名：`/qa`、`/docs`
- action id：`rag_answer`

同时补一条默认回退规则：

- 用户输入普通文本
- 如果当前处于 `@token` 文件模式，不走问答
- 如果当前是显式 slash 输入，仍按 slash 动作处理
- 如果应用搜索弹出了补全框，默认主动作仍是应用启动
- 只有应用搜索没有弹出补全框时，默认主动作回退为 `rag_answer`

这样设计的理由：

- 不抢走已有应用命中
- 能复用现有 matcher -> execute_action -> LauncherFeedback 链路
- 保留显式 `/ask`，用户在需要时仍能强制走问答
- “没有应用候选时默认问答”更符合“launcher 直接提问”的预期

不推荐把所有普通自然语言无条件转成问答。那会直接吃掉应用搜索。

### 2. 结果展示

答案正文继续走现有 `ExecutionResult.primary_text`，并使用 Markdown 渲染：

- `primary_text`：LLM 生成的 Markdown 答案
- `secondary_text`：例如“已检索 5 个片段，命中 3 个文件”
- `structured_payload.render = "markdown"`

但引用链接不要只靠 Markdown 内联链接。原因很简单：

- 现有 Markdown 渲染器对非 `http(s)` 链接不会点击打开
- `file://` 链接在不同平台和 WebView 里行为不稳定

所以引用应拆成独立结构化数据，由前端单独渲染“References”面板。

建议 payload 结构：

```json
{
	"kind": "rag_answer",
	"render": "markdown",
	"reasoning": "可选的次级思考内容",
	"conversationState": {
		"previousResponseId": "resp_123",
		"continuationScope": "scope_hash",
		"citations": [],
		"actions": [],
		"toolCalls": []
	},
	"citations": [
		{
			"id": 1,
			"absolutePath": "/abs/path/file.md",
			"relativePath": "notes/file.md",
			"sourceRoot": "/abs/path",
			"chunkIndex": 3,
			"score": 0.8231,
			"snippet": "..."
		}
	],
	"retrieval": {
		"query": "用户问题",
		"matchCount": 5,
		"fileCount": 3
	}
}
```

补充约束：

- `conversationState` 不是裸 `response_id`；它必须同时带续链 scope 和累计证据链，避免继续追问后 citation / tool trace 丢失
- `reasoning` 是可选次级字段，只在 provider 能同时给出明确正文和 reasoning 时回传；前端只能把它渲染成折叠的 thought disclosure，不能再把它直接当正文兜底展示
- 后端只会在 scope 与当前 provider + workspace 一致时沿用这份状态；scope 不匹配时必须把旧状态和旧历史一起丢弃，避免跨模型或跨 workspace 误续链
- 如果 `responses stateful` 的首轮续问因为 provider 预算或累计上下文过大被拒绝，后端应丢弃旧 `response_id`，回退到显式最近历史再重试一次；否则长 response chain 会把后续问答直接锁死

前端显示策略：

- 上方：答案 Markdown
- 下方：`References` 列表
- 每条引用显示 `[1] relativePath`、命中片段摘要、可选相似度
- 点击引用后通过新命令打开本地文件

答案正文里仍然可以鼓励 LLM 使用 `[1]`、`[2]` 这类编号引用，但真正可点击的引用源以 `structured_payload.citations` 为准。

## 后端链路设计

### 1. 新动作

在 matcher 中新增：

- `title`: `文档问答`
- `summary`: `基于本地 RAG 索引回答问题`
- `aliases`: `/ask` `/qa` `/docs`
- `category`: `knowledge`
- `supported_input_modes`: `inline`、`multiline`、`clipboard`、`selection`

### 2. 执行入口

不要把 RAG 问答塞进纯 `ExecutorService`。

理由：

- 它依赖运行时配置
- 它依赖 LanceDB
- 它依赖网络 LLM 请求
- 它依赖 settings 中的 provider 选择

所以它应和翻译一样，走 `AppState::execute_action` 的专门分支：

- `translate_text` 已经是现成模式
- `rag_answer` 应复用同类调度方式，但运行时分支只做依赖装配，真实问答入口下沉到独立模块 `question_answer_backend`

推荐新增服务：

- `src-tauri/src/services/question_answer_backend.rs`
- `src-tauri/src/services/rag_answer.rs`

不要继续把问答逻辑堆进现在已经很重的 `rag.rs`，也不要让 `AppState` 直接成为唯一可调用入口；否则集成测试只能依赖 UI/Tauri 运行时。

### 3. 查询检索

检索步骤：

1. 校验 RAG 已配置扫描目录和 embedding 条目
2. 校验 AI 功能页已经选择可用于生成答案的问答 LLM 条目
3. 用 RAG 的 embedding 模型对用户问题生成 query embedding
4. 在 LanceDB 对 `vector` 列做 top-k 相似搜索
5. 读取候选 chunk 的 `absolute_path`、对外展示用 `path`、`chunk_index`、`line_start`、`line_end`、`paragraph_line_start`、`heading_path`、`text`
6. 做一次轻量重排和裁剪，再送给 LLM；裁剪不只看固定 `top_k`，还要同时过滤低于默认高置信阈值、明显落后于首个命中的弱相关尾部、以及不满足强实体锚点词约束或命中标题-only / base64 低质量 chunk 的结果

建议的 v1 参数：

- 初始召回 `top_k = 12`
- 每个文件最多保留 `2` 个 chunk，避免单文件霸榜
- 最终送入 LLM 的 chunk 数 `4~6`
- 文本总预算控制在 `4_000 ~ 8_000` 字符

不建议 v1 就引入 reranker 模型。你现在缺的是可用闭环，不是多一层复杂度。

### 4. 相似度过滤

必须有“无结果/低置信度”分支。

否则模型会拿低相关上下文胡编。

建议：

- 检索结果为空：直接返回 warning
- 最高分低于阈值：返回“未找到足够相关文档”
- 即使 `top_k` 还有空位，也不要为了凑数把低相关尾部带进来；需要有相对首命中的尾部截断
- 当 query 本身包含明确实体锚点时，候选至少要满足路径/标题锚点命中，或正文命中足够多的 query term；不能让纯向量相近但完全不含锚点的文档混进来
- 标题-only、base64/密钥块这类低质量 chunk 不应以普通正文证据同权参与最终返回
- 阈值不要写死在 UI，放在后端常量

v1 阈值可以先做静态配置，后续再按模型/距离度量调参。

### 5. 生成答案

答案生成使用 AI 功能页里显式选择的问答 LLM 条目，而不是 Embedding 条目。

当前实现把问答模型选择放在统一的 AI 功能页里，字段名是 `llm.questionAnswerProviderId`，不再复用“默认 LLM”语义。

系统提示词应明确约束：

- 以服务注入的工具结果为准回答
- 需要时允许多轮查询：先检索，再读精确文件或行，直到证据足够
- 可以补充少量合理背景知识，但不能虚构具体文件内容、路径、API 或配置值
- 检索结果冲突或不足以支撑关键细节时必须明确说明
- 需要在相关句子后尽量加 `[1]` 这类编号
- 返回 Markdown，不要输出 JSON

建议传给模型的上下文格式：

```xml
<question>
用户问题
</question>

<doc path="~/notes/arch.md" chunk_index="3" line_start="40" line_end="57" paragraph_line_start="38" heading_path="Architecture > Storage">
...
</doc>

<doc path="/abs/path/notes/api.md" chunk_index="1" line_start="12" line_end="24" paragraph_line_start="12">
...
</doc>
```

### 6. 引用链接打开

需要新增一个后端命令，例如：

- `open_document_reference(path: String)`

行为：

- 桌面端调用系统 opener 打开文件
- 后续可扩展为“在 Finder 中显示”或“用默认编辑器打开”

不要把本地文件打开能力塞进 Markdown 超链接处理里。那层现在就是渲染器，不是安全边界。

## 前端实现建议

### 1. Launcher

`LauncherPage` 无需新开页面，只要复用现有结果卡即可。

需要改的点：

- 让 `/ask` 进入候选列表
- 当普通文本输入未进入 `@token` / slash 模式，且应用搜索没有可见补全项时，把主动作解析为 `rag_answer`
- 当应用搜索存在可见补全项时，仍保持当前应用启动优先级
- 执行成功后，`setResult(executionResult)`
- `LauncherFeedback` 检查 `structuredPayload.kind === "rag_answer"`
- 在答案卡下方额外渲染 `References` 区域

### 2. 引用组件

建议新增一个小组件，例如：

- `components/RagCitationList.tsx`

职责：

- 渲染引用列表
- 点击后触发 `openDocumentReference`
- 不在这里做 Markdown 解析

### 3. MarkdownRenderer

不要让它承担本地文件打开逻辑。

原因：

- 现在的职责是“渲染 Markdown”
- 让它感知桌面 opener 会污染边界
- 引用链接已经有结构化数据，更适合单独组件渲染

## 领域模型建议

当前 `ExecutionResult.structured_payload` 还是弱类型 JSON。  
如果只加一个新动作，v1 可以继续沿用这个做法；但要约定稳定字段：

- `kind = "rag_answer"`
- `render = "markdown"`
- `citations = [...]`
- `retrieval = {...}`

如果后续再增加更多结构化输出类型，再考虑把 `structured_payload` 提升成显式 enum。

## 失败路径

必须覆盖这些情况：

- RAG 索引不存在
- RAG 配置为空
- embedding 条目缺失或非法
- 问答 LLM 缺失或配置非法
- LanceDB 查询失败
- embedding 请求失败
- LLM 作答失败
- 检索结果不足

返回策略：

- 可恢复配置问题：`warning` 或 `error`，直接告诉用户去设置页修
- 无检索结果：`warning`
- 网络/供应商错误：`error`

## 分阶段实施

### Phase 1

- `/ask` 动作
- “应用搜索无补全框时默认回退问答”的主动作切换
- 向量检索
- 单轮答案生成
- 引用列表展示
- 点击打开文件

### Phase 2

- 可调 `top_k` / 阈值
- 每文件去重策略调优
- 高亮命中片段
- 支持“在 Finder 中显示”

### Phase 3

- workspace 作用域过滤
- reranker
- 文档片段预览弹层

## 明确不做

- 不把它做成 ACP session 替代品
- 不在第一版加 reranker
- 不在第一版支持 PDF 抽取
- 不在第一版支持旧二进制 `.doc` 抽取
- 不把所有普通文本无条件改成文档问答；只有应用搜索无补全框时才默认回退
- 不在第一版把引用打开逻辑埋进 Markdown 渲染器

## 状态

- 2026-03-17：完成 v2 实现。问答已切到 `responses + tools`，默认注入 `wabity.rag.query` / `wabity.read_file_lines` 和全局 HTTP/SSE MCP server；launcher 也会回传最近多轮问答历史
- 2026-03-17：问答请求显式发送 `stream=false`，并在兼容 provider 仍返回 SSE 事件流时回退解析 `response.completed`，避免状态栏只报 JSON 解析失败
- 2026-03-17：`stdio` MCP 仍未接入问答链路；当前会跳过这类 server，保留给 ACP session 使用
- 2026-03-18：收敛默认问答系统提示词，核心只保留“工具结果优先、多轮查证、禁止编造、证据不足直说”，减少和运行时规则的重复
- 2026-03-18：问答协议扩成 `chat/completions`、`responses stateless` 和 `responses stateful` 三条链路；`chat/completions` 继续支持内置工具，但不再注入 MCP server
- 2026-03-21：`responses stateful` 在首轮续问遇到 provider budget/context 限制时，会自动丢弃旧 `response_id`，回退到显式最近历史重试一次，降低长链续问失败概率
- 2026-03-25：新增 `question_answer_backend` 公开入口；当前已用本地 mock `chat/completions` server 补上脱离 `AppState` 的问答后端集成测试，验证 builtin `wabity.read_file_lines` 工具回路
- 2026-03-26：RAG 建索引已先支持 `.docx`；索引侧会把 `docx` 规范化成 Markdown 风格文本后再分块，问答里的 `wabity.read_file_lines` 也同步复用这条抽取逻辑回读规范化文本
