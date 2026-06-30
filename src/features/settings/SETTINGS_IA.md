# Settings Information Architecture

## 一级导航

设置页一级导航固定为：

```text
通用 / 功能 / 模型 / 知识库 / 扩展 / 关于
```

技术词如 RAG、MCP、Agent、protocol、embedding 可以保留在副标题、帮助文本、校验消息和设计文档里；不要作为日常设置的一层主导航标签。

## 分组职责

| Section | Responsibility                                                                                |
| ------- | --------------------------------------------------------------------------------------------- |
| 通用    | Appearance、快捷键、通知、开机启动、Dock / 窗口行为。                                         |
| 功能    | Feature bindings：翻译、文档问答、OCR provider/model、提示词。                                |
| 模型    | Provider resources：Base URL、API key、protocol、模型目录、模型类型、多模态 / stateful 能力。 |
| 知识库  | Local document knowledge base：Embedding、扫描目录、忽略规则、索引重建和扫描状态。            |
| 扩展    | Runtime extensions：内嵌 Agent runtime 说明、内置 MCP、自定义全局 MCP 服务。                  |
| 关于    | 版本、项目信息、依赖和更新信息。                                                              |

## 依赖模型

上游资源从 `模型` 流向下游能力：

```text
模型
  -> 功能: translation, document QA, OCR consume LLM / vision-capable models
  -> 知识库: indexing consumes embedding models
  -> Agent: embedded session may reuse the document QA model when safely bridgeable
```

MCP 是运行时扩展路径，不是模型配置路径：

```text
扩展 / MCP
  -> serves global MCP management
  -> may be bridged into Agent sessions later through an explicit Pi SDK ToolFactory design
  -> is not automatically injected into Agent sessions in the current phase
```

## 当前实现约束

- 用户可见 `扩展` 仍由内部 `mcp` section id 承载；旧前端 `acp` settings section id 已从导航类型移除。
- 后端 ACP 兼容命名不由本次信息架构调整改变。
- OCR 属于 `功能` 页，不属于 `通用` 页。
- Agent runtime 是只读说明面板，不是和功能 / 模型 / 知识库等价的可编辑配置页。
- 全局 MCP 清单当前不自动注入 Agent session。

## 页面结构模式

可编辑分组遵循同一结构：

1. Header：一句话说明该分组控制什么。
2. Status / dependency strip：saved、draft、error、跨页依赖或运行时回退状态。
3. Main edit area：真实字段。
4. Action row：保存、恢复、定位问题。
5. Advanced / details：折叠或视觉次要。

摘要卡只在两类场景成立：

- 展示跨分组依赖状态或运行风险。
- 展示用户在编辑当前字段前必须知道的配置边界。

不要用摘要卡重复表单里已经可见的值。

## Quick jump 和导航规则

- 一级导航用真实 `tablist` / `tab` / `tabpanel` 语义，支持方向键、`Home`、`End` 切换。
- 左侧导航只负责分组切换，不跟随右侧配置滚动。
- 右侧 quick jump 只在当前分组存在 2 个及以上、且确实会离开当前视口的编辑块时显示；只有 1 个目标时隐藏。
- quick jump 使用短标签和紧凑 pill，不堆成标签云。
- 主内容区顶部只保留一层 section header：分组标题、简短说明和必要 quick jump。
- 非激活分组不常驻 DOM；切换分组时只渲染当前 `tabpanel`。

## 当前分组细节

### 通用

- 承载开机启动、Dock、语言、快捷键、通知、外观。
- OCR 不在通用页。
- 通用 / 外观配置可实时持久化。

### 功能

- 承载翻译、文档问答、OCR。
- 顶部展示翻译、文档问答和 OCR 的只读依赖健康摘要。
- 翻译和文档问答各自独立保存；OCR 使用独立保存动作。

### 模型

- 维护多个 provider 组。
- Provider 层保存 Base URL、API key、protocol。
- Model 层保存 `id`、`modelType`、`model`、`modelIdentityHint`、`builtinPresetModelId`、`supportsMultimodal`、`supportsStateful`。
- 模型页不承载“默认 LLM”逻辑；翻译 / 文档问答选择在功能页。

### 知识库

- 维护 Embedding、扫描目录、忽略规则、手动重建。
- 顶部展示 Embedding 依赖健康摘要和索引重建提示。
- 有扫描目录时必须选择可用 Embedding 条目。

### 扩展

- 顶部展示 Agent 单运行时说明。
- 下方维护内置 MCP 和自定义全局 MCP 服务。
- Agent 面板不提供保存动作。
- MCP 保存动作在 MCP 主编辑区内，校验错误定位到第一个非法字段。
