# LLM Models Settings

## 背景

- 上一版设置页允许单个 provider 条目同时保存 `responsesModel` 和 `embeddingModel`
- 这让“一个条目到底代表接入点，还是代表具体模型”变得模糊；前端编辑流程和后端消费链路都需要额外猜用途
- 现在结构已经收敛为“provider 分组 + provider 下 `models[]`”；用户仍然需要模板帮助来补齐供应商注册入口、API key 页面和默认 `Base URL`，见 [内置供应商模板方案](./llm-builtin-provider-catalog.md)

## 决策

- LLM 配置固定拆成两层：
  - provider 层：
    - `baseUrl`
    - `apiKey`
    - `protocol`
  - model 层：
    - `id`
    - `modelType`
    - `model`
    - `modelIdentityHint`
    - `builtinPresetModelId`
    - `supportsMultimodal`
    - `supportsStateful`
- `LlmProviderConfig` 是 provider 分组容器；一个 provider 下可以维护多个模型
- 具体模型才是最小配置单元；翻译、问答、OCR、RAG 都按 model id 引用，不再按 provider id 引用
- 设置页编辑顺序固定为：
  - 先选或新建 provider 组
  - 在“供应商预设与连接”里填写 provider 层名称、Base URL、API key
  - 在“模型、调用方式与用途”里切换/新增具体模型，并填写当前模型的模型名与能力
  - “配置类型”直接合并 `modelType + protocol + supportsStateful`
  - 当前配置类型固定区分 `LLM · responses stateless`、`LLM · responses stateful`、`LLM · chat/completions` 和 `Embedding`
  - `responses` 页面仍可额外配置 `supportsMultimodal`；`supportsStateful` 由配置类型本身决定
- LLM 页只维护 provider 组与组内模型本身，不再维护“默认 LLM”概念
- AI 功能页显式维护两个普通 LLM 模型引用：
  - `translationModelId`
  - `questionAnswerModelId`
- OCR 读取 `llmModelId`，要求该模型满足“普通 LLM + protocol=responses + supportsMultimodal = true”
- 翻译直接读取 `translationModelId`；所选模型若是 `responses` 则走 `/responses`，若是 `chat/completions` 则走 `/chat/completions`，运行时不再跨协议兜底
- RAG 问答读取 `questionAnswerModelId`；`responses` 支持 stateful/stateless 两条链路，`chat/completions` 固定走 stateless，运行时不再跨协议兜底
- RAG 建索引和检索读取 `embeddingModelId`

## 兼容策略

- 不再维护 LLM 旧配置迁移。配置读取只接受当前 provider 分组、`models[]` 和 `*ModelId` 引用。
- 旧版平铺模型字段、`*ProviderId` 路由字段、`defaultProviderId`、`responsesModel + embeddingModel` 拆分逻辑已清理。

## 进度

- 2026-03-18：完成单模型条目改造；设置页改成“先选类型再填模型”，共享类型、Rust 归一化、OCR/翻译/RAG 问答/RAG embedding 消费点已同步迁移
- 2026-03-18：补充普通 LLM 条目的 `protocol` 字段；RAG 问答同时支持 `chat/completions`、`responses stateless` 和 `responses stateful`
- 2026-03-19：设置页把 `modelType`、`protocol` 和 `supportsStateful` 合并成单一“配置类型”选择器，直接把 `responses stateful/stateless` 作为不同页面展示
- 2026-03-19：翻译链路补齐 `chat/completions`；默认普通 LLM 条目现在会按自身协议直接发到 `/responses` 或 `/chat/completions`，不再只接受 `responses`
- 2026-03-21：取消 LLM 页里的默认条目逻辑；翻译 LLM 和问答 LLM 改由 AI 功能页显式选择
- 2026-04-22：`LlmProviderConfig` 进一步拆成 provider 层和 `models[]`；翻译、问答、OCR、RAG 全部切到按 `modelId` 配置与引用，provider 只保留连接信息
- 2026-04-23：清理 LLM 旧配置迁移路径，不再接受旧版平铺模型字段或 provider id 路由引用
- 2026-04-23：模型接入页面收敛为两段式主流程，目录仍按 provider 组选择，编辑区合并供应商预设、连接信息、模型切换、调用方式和用途状态，减少同权重面板和重复说明
