# infrastructure

职责：

- `autostart`：同步 `general.autoStart` 与系统登录启动项状态
- `window`：管理 launcher 主窗口的显示、隐藏、聚焦与平台窗口行为
- `hotkey`：解析、注册、注销全局快捷键
- `config`：集中处理本地配置、workspace 历史读写与缓存
- `openai_compatible`：集中封装 OpenAI-compatible 的公共传输细节，包括 base URL 归一化、错误体提取、`responses`/`chat` 文本提取、SSE fallback 解析与 `/models` 列表提取

关键约束：

- `autostart` 只负责把配置态投影到平台登录启动能力，不负责决定“是否应该开启”；开启与关闭仍由设置页和配置模型驱动
- `autostart` 同步分两条链路：启动时做一次配置与系统状态对账，设置保存时对变更即时生效；启动阶段失败只记日志，不阻塞主窗口拉起
- `window` 只负责窗口壳层行为，不承载业务状态或动作匹配逻辑
- `config` 统一将用户配置写入用户配置目录下的 `wabity/config.toml`，其中包含翻译提示词、ACP agent 与全局 MCP 清单；workspace 最近目录历史写入 `wabity/workspace-history.toml`
- `config` 里的快捷键字段虽然落盘时仍是 `toggle_launcher` / `ocr_capture` / `ocr_translate` 三个稳定键名，但运行时访问统一经 `ShortcutKey + ShortcutConfig::{get,set}`，不要在其他模块重复手写字符串分发
- `config` 负责 TOML 序列化、原子 `safe_write`、磁盘读写和内存缓存；启动时由 `AppState::new` 先读取，再把配置投影到运行时状态
- `openai_compatible` 只负责公共协议兼容和响应解析，不承载业务级 prompt、工具编排或 provider 选择；问答、翻译、OCR、RAG embedding 仍各自保留自己的请求体和重试策略
- 配置模型新增字段时必须保持向后兼容；旧版 `config.toml` 缺字段时应通过 `serde(default)` 回填，而不是在启动阶段直接解析失败
- macOS 下 launcher 启动时会把主窗口转换成 `NSPanel`，并配置为 `nonactivating panel`；再叠加 `can_join_all_spaces + full_screen_auxiliary + stationary` 和高层级，这是覆盖全屏 Space 的主实现，不再把普通 `NSWindow` 的补丁当成可靠方案
- launcher 每次显示前都会优先按鼠标当前所在显示器选择目标屏幕，再按该显示器 `work_area` 计算默认位置，而不是依赖配置里的原生 `center`：水平居中，垂直中心落在可用高度 `0.382` 处
- 前端自动测量会继续回写真实窗口尺寸；`window` 模块在 launcher 隐藏期间会持续按默认规则纠正位置，并在每次重新显示后的一个短暂稳定窗口里继续接住首轮 resize，避免首帧宽高变化把水平居中或垂直落点打偏；稳定窗口结束后，后续内容 resize 不再抢位置
- 任意 `resize_main_window` 调用都必须先按当前窗口所在显示器的 `work_area` 裁剪目标宽高；如果 resize 后当前位置会越界，还要继续把窗口位置钳回可见区域，不能只改尺寸不改位置
- 运行时平台特性配置必须在主线程执行，避免直接从普通线程调用 AppKit
- macOS 下命中 `NSPanel` 的显隐、置前和尺寸调整统一通过 Tauri `run_on_main_thread` 调度；后台 OCR 任务结束后也只能经这条通道回到窗口层
- launcher 显隐不依赖底层 `is_visible()` 查询，而是维护独立运行时状态；全局快捷键切换按“按下一次只触发一次，直到收到 Released 才重新解锁”的状态机处理，避免同一轮组合键被重复 toggle
- 当前全局快捷键分成三类：launcher 唤起、截图 OCR 回填，以及“优先翻译当前选中文本；没有选中内容时再截图 OCR 并翻译”；涉及截图的两条链路共享同一个“只允许单次截图流程在跑”的运行时锁，避免并发截图互相踩状态。macOS 下读取选中文本必须留在快捷键处理线程，不能先 `spawn` 到 Tokio worker 再调用输入模拟
- 快捷翻译窗口事件拆成“开始”和“结果”两段：拿到原文后窗口层先发 `ocr-translation-started` 并立即显示 launcher，等后台翻译完成后再发 `ocr-translation-result`；结果回填不应再次依赖首次显示窗口来驱动 UI
