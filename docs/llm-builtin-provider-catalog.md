# LLM 内置供应商模板方案

## 背景

当前 `LLM` 设置页已经支持：

- 手动新增条目
- 手填 `Base URL / API Key / model`
- 按条目协议选择 `responses / chat_completions / embedding`
- 通过当前条目的 `/models` 拉取远端模型列表

这套能力对“已经理解 OpenAI-compatible 接入细节”的用户够用，但对普通用户不够。

问题不是“不能配”，而是“要先自己研究怎么配”：

- 不知道该去哪个页面注册
- 不知道 API Key 在哪个控制台生成
- 不知道该填哪个 `Base URL`
- 不知道这个供应商该走 `responses` 还是 `chat/completions`
- 不知道哪些模型适合翻译、问答、OCR 或 embedding
- 有些供应商的 `/models` 需要鉴权，用户在填 key 之前拿不到任何引导

用户要求的是一条更短的路径：

1. 选择内置供应商
2. 跟着引导去官网注册
3. 生成并填入 API Key
4. 拉取当前服务真实模型目录；目录不可用时再手填模型
5. 保存后即可被翻译、问答、OCR、RAG 使用

## 目标

- 在 `LLM` 页面提供“内置供应商模板”目录，而不是只允许纯手工配置
- 用户选择模板后，系统自动填充默认 `Base URL`；模型仍以实际服务能力和用户选择为准。若当前模型命中模板目录里的已知元数据，则协议与能力应直接跟随该模型；只有未知模型才退回手工选择
- 每个内置模板显式展示：
  - 注册入口
  - API Key 管理入口
  - 官方文档入口
  - 预置模型列表
  - 每个模型对应的用途和协议限制
- 用户优先通过远端 `/models` 获取当前服务真实可用模型；目录不可用时再手填
- 仍然保留现有手工模式，避免把企业代理、内网网关和自建兼容层挡死

## 非目标

- 不做平台代理转发，不托管用户 API Key
- 不把非 OpenAI-compatible 供应商硬塞进当前链路
- 不在第一期做“自动注册 / 自动创建 API Key”
- 不试图一次性覆盖十几个供应商

这里有一个必须说清的前提：当前运行时和设置模型都围绕 OpenAI-compatible 协议设计。你举的智谱例子符合这个前提，因为它能走 `/chat/completions`。如果后续要接入原生 Anthropic、Gemini、Vertex AI，这不是“再加几个模板”的问题，而是协议层扩展。

## 方案结论

采用“两层模型”：

1. 内置目录层：只描述供应商模板和接入引导，不保存用户密钥
2. 用户配置层：沿用现有 `LlmProviderConfig` 保存真实可用条目和用户 API Key

这样做的原因很直接：

- 内置目录属于应用发布物，应该随版本升级
- 用户条目属于本地私有配置，应该继续写入 `config.toml`
- 两者混在一起会让升级、迁移和密钥存储边界变脏

## 信息架构

### 1. 新增“内置模板”目录

`LLM` 分组保留单一 `新增` 入口。

推荐交互：

- 用户点击 `新增` 后，先得到一个普通 LLM 草稿
- 右侧编辑卡顶部提供 `使用模板` 下拉框
- 选择模板后，当前条目立即被模板填充；切回“不使用模板”时只解除模板来源，不清空当前字段值
- 模板挂载后仍允许继续修改 `配置类型 / model / OCR 能力`；模板不是持续性锁

### 2. 模板卡片字段

每个内置供应商模板至少包含：

- `providerId`
- `displayName`
- `description`
- `registrationLabel`
- `registrationUrl`
- `apiKeyLabel`
- `apiKeyUrl`
- `docsLabel`
- `docsUrl`
- `defaultBaseUrl`
- `supportsModelListing`
- `models[]`

每个内置模型至少包含：

- `modelId`
- `displayName`
- `model`
- `modelType`
- `protocol`
- `supportsMultimodal`
- `supportsStateful`
- `recommendedFor[]`
- `status`
- `summary`
- `selectableInCurrentApp`
- `disabledReason`

`recommendedFor` 只表达产品用途，不表达运行时权限，枚举固定为：

- `translation`
- `rag_answer`
- `ocr`
- `embedding`

`summary` 是内置目录中对模型能力的静态说明，主要用于文档和供应商目录维护，不再直接驱动设置页里的默认模型选择。

`selectableInCurrentApp` 明确这个模型当前能不能进入 Wabity 的模型选择器。

这里补一个之前文档没说清的前提：模板动作按钮不能假设所有供应商都遵循“注册 / API Key / 文档”三段式。像 `Ollama` 这种本地 provider，更合理的是：

- `下载安装`
- `OpenAI 兼容说明`
- `模型 / Embedding 文档`

所以模板元数据需要自带动作标签，而不是前端写死按钮文案。

原因很简单：

- 供应商目录里可以有更多模型
- 但 Wabity 当前运行时只支持有限的任务类型
- 目录可见不等于当前可用

### 3. 智谱模板示例

基于当前需求，第一期至少内置一个智谱模板：

- `providerId`: `zhipu`
- `displayName`: `智谱 AI`
- `registrationUrl`: `https://bigmodel.cn/login?redirect=%2Fusercenter%2Fproj-mgmt%2Fapikeys`
- `apiKeyUrl`: `https://bigmodel.cn/login?redirect=%2Fusercenter%2Fproj-mgmt%2Fapikeys`
- `docsUrl`: `https://docs.bigmodel.cn/cn/guide/start/quick-start`
- `defaultBaseUrl`: `https://open.bigmodel.cn/api/paas/v4`
- `supportsModelListing`: `true`

智谱免费模型目录按 2026-03-23 官方页面列出以下 7 个模型：

1. `glm-4.7-flash`
   - `summary`: 面向 Agentic Coding、长任务规划和工具协同的免费文本模型，适合翻译、问答和复杂对话。
   - `modelType = llm`
   - `protocol = chat_completions`
   - `supportsMultimodal = false`
   - `supportsStateful = false`
   - `recommendedFor = [translation, rag_answer]`
   - `selectableInCurrentApp = true`
2. `glm-4.6v-flash`
   - `summary`: 免费多模态理解模型，支持图像、视频、文件理解和原生工具调用，适合 OCR 与视觉问答。
   - `modelType = llm`
   - `protocol = chat_completions`
   - `supportsMultimodal = true`
   - `supportsStateful = false`
   - `recommendedFor = [translation, rag_answer, ocr]`
   - `selectableInCurrentApp = true`
3. `glm-4.1v-thinking-flash`
   - `summary`: 免费视觉推理模型，擅长图表、视频、GUI 和网页任务，强调更强的可解释推理。
   - `modelType = llm`
   - `protocol = chat_completions`
   - `supportsMultimodal = true`
   - `supportsStateful = false`
   - `recommendedFor = [translation, rag_answer, ocr]`
   - `selectableInCurrentApp = true`
4. `glm-4-flash-250414`
   - `summary`: 智谱首个免费大模型 API，擅长网页检索、长上下文和通用文本处理，适合作为轻量文本问答模型。
   - `modelType = llm`
   - `protocol = chat_completions`
   - `supportsMultimodal = false`
   - `supportsStateful = false`
   - `recommendedFor = [translation, rag_answer]`
   - `selectableInCurrentApp = true`
5. `glm-4v-flash`
   - `summary`: 完全免费的图像理解模型，适合图像识别、图像问答和图像推理。
   - `modelType = llm`
   - `protocol = chat_completions`
   - `supportsMultimodal = true`
   - `supportsStateful = false`
   - `recommendedFor = [translation, rag_answer, ocr]`
   - `selectableInCurrentApp = true`
6. `cogview-3-flash`
   - `summary`: 免费图像生成模型，适合把文本指令快速转成高美感图片。
   - `modelType = image_generation`
   - `protocol = unsupported`
   - `recommendedFor = []`
   - `selectableInCurrentApp = false`
   - `disabledReason = 当前 Wabity 没有图像生成调用链路，不能把它当普通 LLM 使用。`
7. `cogvideox-flash`
   - `summary`: 免费视频生成模型，适合根据文本指令生成视频。
   - `modelType = video_generation`
   - `protocol = unsupported`
   - `recommendedFor = []`
   - `selectableInCurrentApp = false`
   - `disabledReason = 当前 Wabity 没有视频生成调用链路，不能把它当普通 LLM 使用。`

注意两点：

- 这里把智谱文本/视觉理解模型统一按 `chat_completions` 模板接入，因为你给的官方示例就是 `POST /chat/completions`。
- `CogView-3-Flash` 和 `CogVideoX-Flash` 必须进入目录，但不能在当前 `LLM` 选择器里冒充可用于翻译、问答、OCR 或 RAG 的模型。

### 4. SiliconFlow 模板示例

第二个内置模板接入 SiliconFlow：

- `providerId`: `siliconflow`
- `displayName`: `SiliconFlow`
- `registrationUrl`: `https://account.siliconflow.cn`
- `apiKeyUrl`: `https://cloud.siliconflow.cn/account/ak`
- `docsUrl`: `https://docs.siliconflow.cn/cn/api-reference/chat-completions/chat-completions`
- `defaultBaseUrl`: `https://api.siliconflow.cn/v1`
- `supportsModelListing`: `true`

SiliconFlow 免费语言模型目录按 2026-03-23 官方定价页列出以下 13 个模型：

1. `Qwen/Qwen3.5-4B-Instruct-2507`
2. `PaddlePaddle/PaddleOCR-VL-1.5`
3. `deepseek-ai/DeepSeek-R1-Distill-Qwen-7B`
4. `THUDM/GLM-4.1V-9B-Thinking`
5. `PaddlePaddle/PaddleOCR-VL`
6. `deepseek-ai/DeepSeek-OCR`
7. `Qwen/Qwen3-8B`
8. `tencent/Hunyuan-MT-7B`
9. `deepseek-ai/DeepSeek-R1-0528-Qwen3-8B`
10. `THUDM/GLM-Z1-9B-0414`
11. `Qwen/Qwen2.5-7B-Instruct`
12. `THUDM/GLM-4-9B-0414`
13. `internlm/internlm2_5-7b-chat`

设计约束：

- 当前先按 `chat_completions` 模板接入，因为 SiliconFlow 官方文档主链路就是 OpenAI 兼容 `chat/completions`
- 这些模型里存在 OCR / 视觉理解模型，但 Wabity 当前 OCR 入口仍要求 `responses`；因此这里可以把它们作为普通 LLM 条目创建，用于翻译或问答试配，但不会自动进入 OCR 可选列表
- 只收录官方定价页当前明确标为免费的语言模型；未标免费、已下线或不在当前页面中的模型不进白名单

### 5. OpenAI / OpenRouter / DeepSeek / Ollama 模板补充

当前目录已额外补四类常见供应商：

- `OpenAI`
  - `defaultBaseUrl = https://api.openai.com/v1`
  - 常用模型目录覆盖 `gpt-5.4`、`gpt-5.4-mini`、`gpt-5.4-nano`、`text-embedding-3-small`、`text-embedding-3-large`
  - 其中通用模型按 `responses` 模板接入，并允许声明多模态和 stateful；Embedding 条目只进入 RAG
- `OpenRouter`
  - `defaultBaseUrl = https://openrouter.ai/api/v1`
  - 这是聚合网关模板，不再在前端内置推荐模型或白名单；模型一律以当前账号实际拉取到的 `/models` 结果为准
  - 模板只提供注册入口、API Key 页面和官方文档，适合快速填入聚合网关接入点
- `DeepSeek`
  - `defaultBaseUrl = https://api.deepseek.com`
  - 当前目录收录 `deepseek-chat` 和 `deepseek-reasoner`
  - 它们按 `chat_completions` 模板接入，不进入 OCR 列表
- `Ollama`
  - `defaultBaseUrl = http://localhost:11434/v1`
  - 不要求注册，也不强制 API Key；按钮文案应改成安装和兼容说明
  - 目录示例覆盖本地常见的 `qwen3:8b`、`gpt-oss:20b`、`qwen3-vl:8b` 和 `embeddinggemma`
  - 其中视觉模型仍然不会自动进入当前 OCR 列表，因为现有 OCR 入口只接受 `responses + multimodal`

## 配置模型改动

### 1. 领域模型

在 `src-tauri/src/domain/settings.rs` 的 `LlmProviderConfig` 上新增可选来源元数据：

- `builtinPresetId: Option<String>`
- `builtinPresetModelId: Option<String>`
- `managedBaseUrl: bool`

含义：

- `builtinPresetId`：这个条目最初来自哪个内置供应商模板
- `builtinPresetModelId`：当前条目最初来自哪个内置模型模板
- `managedBaseUrl`：当前 `Base URL` 是否仍然跟随模板管理

为什么需要这些字段：

- UI 要能标出“这是智谱模板创建的条目”
- 后续模板升级时，才能判断是否允许安全刷新默认 `Base URL`
- 一旦用户切到高级模式手改接入点，就不能再静默覆盖

### 2. 内置目录结构

新增只读模型，不写入用户配置：

- Rust 侧：
  - `src-tauri/src/domain/settings.rs` 或新增 `src-tauri/src/domain/llm_catalog.rs`
- TypeScript 侧：
  - `src/lib/tauri/types.ts` 增加 `BuiltinLlmProviderTemplate`
  - `src/lib/tauri/client.ts` 增加 `listBuiltinLlmProviderTemplates`

不建议把目录只写在前端常量里。原因：

- 运行时校验和 UI 展示会出现双份真相
- 未来如果 launcher、设置页、CLI 都要复用模板，前端常量会马上失控

## IPC 与后端

新增命令：

- `listBuiltinLlmProviderTemplates() -> BuiltinLlmProviderTemplate[]`

Rust 侧职责：

- 返回只读模板目录
- 保证字段完整
- 对外只暴露官方链接和当前版本内置模型元数据

现有 `setAppSettings` 不需要拆接口，只需要接受新增的来源字段。

## 前端交互设计

### 1. 新建入口

在 `LLM` 页已有“新增”按钮旁新增：

- `新增`

新增后在当前条目的编辑卡顶部，通过 `使用模板` 下拉套用内置模板。

### 2. 创建流程

选中模板后，右侧表单仍在当前条目内完成配置：

1. 在 `使用模板` 下拉里选择供应商
2. 跟随右侧官方链接注册并创建 API Key
3. 在同一张编辑卡里填入 API Key
4. 从模板白名单里选择模型并保存

### 3. 表单行为

套用模板后的条目默认行为：

- 自动填入名称
- 自动填入 `Base URL`
- 自动设置 `modelType / protocol / supportsMultimodal / supportsStateful`
- 自动绑定 `builtinPresetId / builtinPresetModelId`

`Base URL` 默认只读，并显示“由模板管理”。

提供一个显式动作：

- `改为自定义接入点`

点击后：

- `managedBaseUrl = false`
- 输入框变为可编辑
- UI 提示“此条目已脱离模板默认接入点管理”

这是必要的。否则企业代理、转发网关、兼容层用户会被硬拦住。

### 4. 模型选择

模型选择区改成“目录白名单选择”，不再允许任意手填。

规则改成：

1. 内置目录提供当前供应商的白名单模型
2. 下拉框只展示 `selectableInCurrentApp = true` 的模型
3. 每个模型在下拉项中展示：
   - 模型名
   - 一句话说明 `summary`
   - 能力标签，例如 `文本`、`视觉`、`OCR`、`问答`
4. 不再允许自由手填未收录模型
5. `/models` 拉取结果只用于“后台核对模型是否仍存在”，不再扩展可选范围

这样做的效果是明确的：

- 用户只能选产品批准过的模型
- 不会因为供应商返回几十个内部、实验或不兼容模型把下拉框污染掉
- 方案和你的要求一致：只允许使用目录里提供的模型

代价也必须说清楚：

- 白名单维护成本上升
- 供应商新增优质模型后，用户不能立刻使用，必须等应用更新模板目录

这是你要求“只允许使用提供的这几个模型”带来的必然代价，不是实现细节。

### 5. AI 功能页联动

`AI 功能` 页里的翻译 LLM / 问答 LLM 选择器保持不变，但选项文案补充来源信息：

- `智谱 AI · glm-4.7-flash`
- `手动配置 · custom-gateway`

`OCR` 与 `RAG` 选择器同理。

条目创建完成后的模型字段应视为“目录绑定值”，不能再在编辑态随意输入任意模型名；要改模型，只能重新打开该供应商的白名单下拉框选择。

## 校验与约束

### 1. 条目校验

对内置条目增加额外约束：

- 模板目录里的模型说明只能声明当前系统支持的组合，不能再被前端当作默认模型来源
- `ocr` 只能选择 `responses + supportsMultimodal = true`
- `embedding` 只能进入 RAG embedding 列表
- `chat_completions` 不能假装支持 stateful
- 选择器只接受当前模板目录里显式存在且 `selectableInCurrentApp = true` 的模型
- 目录里存在但 `selectableInCurrentApp = false` 的模型必须展示禁用原因，不能静默消失

### 2. 模板校验

应用启动时对模板目录做一次静态校验：

- URL 必须是 `https://`
- `providerId / modelId` 唯一
- 模板模型与运行时能力矩阵一致
- 同一模型的 `recommendedFor` 不得与协议约束冲突
- `summary` 不能为空，且长度限制在适合下拉项展示的一句话范围内
- `selectableInCurrentApp = false` 的模型不得映射到现有 `llm / embedding` 可执行链路

### 3. 保存校验

保存逻辑维持现状，再补三条：

- 如果条目仍声明 `managedBaseUrl = true`，则 `baseUrl` 必须等于模板默认值
- 如果 `builtinPresetModelId` 存在，但用户手动改了协议类型，则应清空 `builtinPresetModelId`
- 如果条目来自内置模板，则 `model` 必须属于该模板白名单且当前可选

原因：来源元数据只能反映真实来源，不能让 UI 长期显示伪关联。

## 安全与隐私

这里有一个现有缺口，不能装作不存在：

- 现在 `apiKey` 仍然写在本地 `config.toml`

内置模板不会让这个问题更严重，但也没有解决它。

所以方案要明确记录：

- 第一期沿用现状，只改善接入体验
- 第二期再评估迁移到系统钥匙串：
  - macOS Keychain
  - Windows Credential Manager
  - Linux Secret Service

如果不把这点写清楚，后续“内置供应商更方便”会掩盖“密钥仍是明文本地存储”的事实。

## 数据迁移

老配置兼容原则：

- 已存在手工条目不做破坏性迁移
- 新增字段全部可选
- 旧条目默认：
  - `builtinPresetId = null`
  - `builtinPresetModelId = null`
  - `managedBaseUrl = false`

不会尝试自动猜测旧条目来自哪个供应商模板。原因很简单：猜错就会污染用户配置来源。

## 测试方案

### Rust

单元测试：

- 模板目录静态校验
- `managedBaseUrl` 约束校验
- 模板模型与协议矩阵校验
- 旧配置反序列化兼容

集成测试：

- `listBuiltinLlmProviderTemplates` 返回结构稳定
- `setAppSettings` 接受模板来源字段并正确持久化

### Frontend

组件测试：

- `使用模板` 下拉渲染与切换
- 套用模板后字段自动填充
- 切换到“自定义接入点”后 `Base URL` 可编辑
- 模型下拉框只显示白名单模型
- 每个下拉项都展示一句话说明
- 不可选模型显示禁用原因或只在目录详情中展示

交互测试：

- 智谱模板完整流程
  - 打开注册链接
  - 回填 API Key
  - 选择 `glm-4.7-flash`
  - 保存后可在 AI 功能页被选中

回归测试：

- 纯手工新增条目流程不退化
- 现有 `/models` 拉取逻辑不退化
- OCR / 翻译 / RAG 引用逻辑不受影响

## 实施阶段

### Phase 1: 最小可用

- 新增模板目录只读接口
- LLM 页新增“从内置模板创建”
- 支持 OpenAI / DeepSeek / Ollama / 智谱 / SiliconFlow 模板
- 收录智谱免费模型页当前 7 个模型
- 收录 SiliconFlow 定价页当前 13 个免费语言模型
- 收录 OpenAI / DeepSeek / Ollama 的常用模型白名单
- 其中仅允许当前应用可消费的模型进入选择下拉框
- 模型下拉项展示一句话说明
- 模板动作按钮文案跟随供应商元数据
- 保存来源元数据

验收标准：

- 新用户无需理解 `Base URL / protocol` 就能配好一个智谱条目
- 用户无法绕过下拉框选择目录外模型

### Phase 2: 补齐产品化体验

- 模板筛选与搜索
- 来源徽标与只读引导
- `改为自定义接入点`
- AI 功能页文案联动

### Phase 3: 安全与维护

- API Key 存储迁移评估
- 模板版本化与升级策略
- 增加更多兼容供应商模板

## 风险

### 1. 供应商信息漂移

注册链接、控制台路径、默认 `Base URL`、模型名都可能变化。

缓解：

- 模板目录跟随应用版本发布
- 模板卡片显式标“最后校验日期”
- 只放官方链接，不放二手教程

### 2. 预置模型过期

有些模型会下线或更名。

缓解：

- 预置模型只是推荐，不是唯一来源
- 仍允许 `/models` 拉取和手工输入

### 3. 用户误以为“内置模板 = 官方保证可用”

这是假设错误。

应用只能保证：

- 帮用户填对接入参数
- 帮用户跳到官方页面

不能保证：

- 用户账号一定已开通对应模型
- 该模型在用户所在地域、套餐、权限下可用

UI 必须写清楚“是否可调用仍取决于供应商账户权限”。

## 推荐落地文件

- 新增文档：
  - `docs/llm-builtin-provider-catalog.md`
- Rust：
  - `src-tauri/src/domain/settings.rs`
  - `src-tauri/src/commands/settings.rs`
  - `src-tauri/src/state/mod.rs`
- Frontend：
  - `src/lib/tauri/types.ts`
  - `src/lib/tauri/client.ts`
  - `src/features/settings/SettingsPage.tsx`
  - `src/features/settings/settings.css`

## 进度

- 2026-03-23：完成完整实现方案设计，确定采用“内置目录层 + 用户配置层”双层结构；首期先落智谱模板验证链路，再按白名单继续扩供应商
- 2026-03-23：落地首期实现：Rust/前端已接入智谱内置模板目录、白名单模型选择、模型一句话说明，以及“目录可见但当前不可选”的禁用模型展示
- 2026-03-23：扩展第二批内置模板，新增 SiliconFlow 官方免费语言模型目录，来源对齐 `https://www.siliconflow.cn/pricing#bmf0`
- 2026-03-31：扩展常见供应商模板，新增 OpenAI / DeepSeek / Ollama；同时把模板动作按钮改成供应商自定义标签，避免本地 provider 被误渲染成“注册 / API Key”流程
