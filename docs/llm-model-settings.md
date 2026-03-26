# LLM Models Settings

## 背景

- 上一版设置页允许单个条目同时保存 `responsesModel` 和 `embeddingModel`
- 这让“一个条目到底代表接入点，还是代表具体模型”变得模糊；前端编辑流程和后端消费链路都需要额外猜用途
- 当前条目模型已经按“一个条目只绑定一个模型”收敛，但用户首次接入仍然需要自己研究供应商注册入口、API key 页面、默认 `Base URL` 和推荐模型，见 [内置供应商模板方案](./llm-builtin-provider-catalog.md)

## 决策

- 每个模型条目只配置一个模型，不再在同一条目里混放普通 LLM 和 embedding
- 每条配置统一保存：
  - `baseUrl`
  - `apiKey`
  - `modelType`
  - `protocol`
  - `model`
  - `supportsMultimodal`
  - `supportsStateful`
- 设置页编辑顺序固定为：
  - 先选“配置类型”
  - 再填写名称、Base URL、API key 和模型名
  - “配置类型”直接合并 `modelType + protocol + supportsStateful`
  - 当前配置类型固定区分 `LLM · responses stateless`、`LLM · responses stateful`、`LLM · chat/completions` 和 `Embedding`
  - `responses` 页面仍可额外配置 `supportsMultimodal`；`supportsStateful` 由配置类型本身决定
- LLM 页只维护条目本身，不再维护“默认 LLM”概念
- AI 功能页显式维护两个普通 LLM 引用：
  - `translationProviderId`
  - `questionAnswerProviderId`
- OCR 读取“普通 LLM + protocol=responses + supportsMultimodal = true”
- 翻译直接读取 `translationProviderId`；所选条目若是 `responses` 则走 `/responses`，若是 `chat/completions` 则走 `/chat/completions`，运行时不再跨协议兜底
- RAG 问答读取 `questionAnswerProviderId`；`responses` 支持 stateful/stateless 两条链路，`chat/completions` 固定走 stateless，运行时不再跨协议兜底
- RAG 建索引和检索读取 Embedding 条目

## 兼容

- 旧配置里的 `protocol`、`model`、`supportsEmbedding` 会在归一化阶段迁移到新字段
- 其中旧版 `openai_chat` / `openai_compatible` 会迁移成 `chat/completions`，`openai_responses` 会迁移成 `responses`
- 旧版同时保存 `responsesModel + embeddingModel` 的单条配置会在归一化阶段拆成两条新配置
- 拆分时会尽量保留原条目 id 给普通 LLM 条目，并把 OCR / 旧版默认 LLM / RAG 的旧引用重定向到对应的新条目

## 进度

- 2026-03-18：完成单模型条目改造；设置页改成“先选类型再填模型”，共享类型、Rust 归一化、OCR/翻译/RAG 问答/RAG embedding 消费点已同步迁移
- 2026-03-18：补充普通 LLM 条目的 `protocol` 字段；RAG 问答同时支持 `chat/completions`、`responses stateless` 和 `responses stateful`
- 2026-03-19：设置页把 `modelType`、`protocol` 和 `supportsStateful` 合并成单一“配置类型”选择器，直接把 `responses stateful/stateless` 作为不同页面展示
- 2026-03-19：翻译链路补齐 `chat/completions`；默认普通 LLM 条目现在会按自身协议直接发到 `/responses` 或 `/chat/completions`，不再只接受 `responses`
- 2026-03-21：取消 LLM 页里的默认条目逻辑；翻译 LLM 和问答 LLM 改由 AI 功能页显式选择，旧 `defaultProviderId` 仅在读取旧配置时作为迁移输入
