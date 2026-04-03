# wabity-openai-compatible

内部 workspace crate，用于承载 OpenAI-compatible 传输与响应解析的公共能力。

边界：

- 负责 base URL 归一化、鉴权注入、HTTP 请求发送
- 负责 `responses` / `chat/completions` 的文本提取与 SSE 归并
- 负责 provider 错误体提取与 `/models` 列表解析

非职责：

- 不依赖宿主 crate 的领域模型
- 不承载问答、翻译、OCR、RAG embedding 的业务请求体、prompt 或重试策略
- 不处理 provider 选择、工具编排或 UI 投影

宿主 crate `src/infrastructure/openai_compatible.rs` 只保留兼容导出，以及到本地 `LlmProviderModelEntry` 的最小转换。
