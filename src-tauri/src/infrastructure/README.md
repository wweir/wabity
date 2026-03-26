# infrastructure

职责：

- `autostart`：同步 `general.autoStart` 与系统登录启动项状态
- `window`：管理 launcher 主窗口的显示、隐藏、聚焦与平台窗口行为
- `hotkey`：解析、注册、注销全局快捷键
- `config`：集中处理本地配置、workspace 历史读写与缓存
- `openai_compatible`：集中封装 OpenAI-compatible 的薄 client 和公共传输细节，包括 base URL 归一化、鉴权注入、请求发送、错误体提取、`responses`/`chat` 文本提取、SSE 流式消费与归并、`/models` 列表提取

关键约束：

- `autostart` 只负责把配置态投影到平台登录启动能力，不负责决定“是否应该开启”；开启与关闭仍由设置页和配置模型驱动
- `autostart` 同步分两条链路：启动时做一次配置与系统状态对账，设置保存时对变更即时生效；启动阶段失败只记日志，不阻塞主窗口拉起
- `window` 只负责窗口壳层行为，不承载业务状态或动作匹配逻辑
- `config` 统一将用户配置写入用户配置目录下的 `wabity/config.toml`，其中包含翻译提示词、ACP agent 与全局 MCP 清单；workspace 最近目录历史写入 `wabity/workspace-history.toml`
- `config` 里的快捷键字段虽然落盘时仍是 `toggle_launcher` / `ocr_capture` / `ocr_translate` 三个稳定键名，但运行时访问统一经 `ShortcutKey + ShortcutConfig::{get,set}`，不要在其他模块重复手写字符串分发
- `config` 负责 TOML 序列化、原子 `safe_write`、磁盘读写和内存缓存；启动时由 `AppState::new` 先读取，再把配置投影到运行时状态
- `openai_compatible` 只负责公共协议兼容、薄传输 client 和响应解析，不承载业务级 prompt、工具编排或 provider 选择；问答、翻译、OCR、RAG embedding 仍各自保留自己的请求体和重试策略，但只要 provider 返回 SSE，就必须由这里按流读取并归并成统一 payload，而不是让上层先把整段 body 读完再猜协议
- 配置模型新增字段时必须保持向后兼容；旧版 `config.toml` 缺字段时应通过 `serde(default)` 回填，而不是在启动阶段直接解析失败
- macOS 下 launcher 启动时会把主窗口转换成 borderless `NSPanel`；为了覆盖全屏 Space，它必须继续叠加 `nonactivating_panel + can_join_all_spaces + full_screen_auxiliary + stationary`，并提升到 `Status` 层级。这里不能只靠普通可激活 panel 去压全屏应用，因为原生 fullscreen 行为本身就要求 auxiliary nonactivating panel
- macOS launcher panel 需要显式开启 `worksWhenModal`；显示时优先走 panel 自己的 `show_and_make_key` / `orderFrontRegardless` 组合，不再额外激活整个 app，否则会把“覆盖当前全屏 Space”退化成“切回 launcher 所在空间”
- launcher 每次显示前都会优先按鼠标当前所在显示器选择目标屏幕，再按该显示器 `work_area` 计算默认位置，而不是依赖配置里的原生 `center`：水平居中，垂直中心落在可用高度 `0.382` 处
- 前端自动测量会继续回写真实窗口尺寸；`window` 模块在 launcher 隐藏期间会持续按默认规则纠正位置，并在每次重新显示后的一个短暂稳定窗口里继续接住首轮 resize，避免首帧宽高变化把水平居中或垂直落点打偏；稳定窗口结束后，后续内容 resize 不再抢位置
- 当 launcher 已可见且前端因为回答渲染、历史更新或布局变化继续回写窗口尺寸时，窗口层仍会重新拉起一小段 `blur` 抑制期，避免内容驱动 resize 后把短暂焦点抖动误判成真实离焦
- `begin/end_transient_window_interaction` 只覆盖“明确知道正在做内部窗口交互”的阶段；问答结果真正落地时，前端还必须显式调用 `arm_launcher_blur_auto_hide_suppression`；若 macOS 结果展示阶段仍持续发生真实失焦，前端会在结果落地与 resize 稳定期内临时关闭 blur auto-hide，并暂停 `window focus` / `visibilitychange` / `onFocusChanged` 这类被动输入框 refocus 链；稳定期结束后这两条保护要自动恢复，显式退出 QA 展示态时也要立即恢复，不能继续把恢复时机绑死在“下一次用户输入”
- QA 结果展示期不能继续让 auto-resize 自由 shrink；但也不能把持续观察彻底停掉，否则首屏回答和延迟渲染内容会被旧窗口高度截断。当前策略是：继续保留持续观察，但把窗口同步切到“只增不减”模式，直到离开 QA 展示态才恢复正常 shrink
- macOS 下 launcher 仍不能把 `Focused(false)` 当成立即关闭信号；即便改成可激活 `NSPanel`，显示后、回答渲染后和空闲阶段仍可能出现无用户操作的假离焦，`NSApp.isActive()` 也不是可靠判据。窗口层不再探测 AppKit `currentEvent` 这类高风险运行时细节，而是统一采用“短暂抑制期 + 延迟确认 + 重新获焦取消”的状态机来过滤假离焦
- macOS 下收到 `Focused(false)` 也不能无条件相信事件本身；在进入确认隐藏前，窗口层还要用 Tauri 的 `window.is_focused()` 再做一次安全复核，避免把明显自相矛盾的假 blur 直接收口成自动隐藏
- 当 blur auto-hide 被前端显式关闭时，窗口层默认只取消确认隐藏并重新拉起一小段 suppression，不主动抢回 key 焦点；继续在 `Focused(false)` 回调里直接 `show` / `makeKey` 会把结果展示期打成原生 `blur/focus` 循环，并留下一个置前但不可输入的 launcher。重新获焦仍交给用户显式编辑、点击或再次触发快捷键
- 在 macOS 的“运行时仍标记 visible，但窗口实际上已经掉出前台”的 re-show 分支里，窗口层也不能偷偷把 blur auto-hide 重置为 `true`；若前端正处于 QA 结果展示期，这个 re-show 只是恢复可见/可聚焦状态，不是退出 QA 展示态
- 但 macOS 下 blur-disabled 阶段也不能只靠前端状态推断窗口还在不在；如果 panel 已经真正掉出可见层，窗口层必须先记录原生 `NSPanel` 的 `isVisible / isKeyWindow / occlusionState / NSApp.isActive`，再只做一次“保持可见、不抢焦点”的补偿：允许 `show + orderFrontRegardless`，禁止重新 `activate` 或 `makeKey`
- macOS 下全局快捷键的 toggle 也不能只看 `launcher_visible` 布尔值；若 `NSPanel` 仍标记可见但已经失焦、失去 key，或 app 已经 inactive，再按快捷键必须重新置前，而不是先执行一次对用户不可见的隐藏
- `toggle_launcher` 热路径只做显隐与置前，不再顺手读取外部应用选中文本；选区读取会触发模拟复制和剪贴板轮询，把它塞进 toggle 只会制造肉眼可见的快捷键延迟
- 所有平台的失焦自动隐藏逻辑都不是“任何 `blur` 立刻收起”；每次显示后的短暂稳定窗口里，窗口层会直接忽略首个瞬时 `Focused(false)`，稳定窗口结束后的后续 `blur` 也要经过很短的确认延迟，若期间焦点恢复则取消隐藏
- 任意 `resize_main_window` 调用都必须先按当前窗口所在显示器的 `work_area` 裁剪目标宽高；如果 resize 后当前位置会越界，还要继续把窗口位置钳回可见区域，不能只改尺寸不改位置
- 运行时平台特性配置必须在主线程执行，避免直接从普通线程调用 AppKit
- macOS 下命中 `NSPanel` 的显隐、置前和尺寸调整统一通过 Tauri `run_on_main_thread` 调度；后台 OCR 任务结束后也只能经这条通道回到窗口层
- launcher 显隐不依赖底层 `is_visible()` 查询，而是维护独立运行时状态；全局快捷键切换仍按“按下一次只触发一次”的门闩处理来避免同一轮组合键重复 toggle，但运行时必须容忍 macOS 丢失 `Released` 事件：若 release 长时间未到，门闩要自动恢复，不能把 launcher / OCR 快捷键永久锁死
- 当前全局快捷键分成三类：launcher 唤起、截图 OCR 回填，以及“优先翻译当前选中文本；没有选中内容时再截图 OCR 并翻译”；涉及截图的两条链路共享同一个“只允许单次截图流程在跑”的运行时锁，避免并发截图互相踩状态。macOS 下读取选中文本必须留在快捷键处理线程，不能先 `spawn` 到 Tokio worker 再调用输入模拟
- 前端驱动的内部窗口尺寸同步也必须显式包进 `begin/end_transient_window_interaction`；否则问答结果或长文本回填触发的 resize 抖动会被错误识别成真实离焦
- 但这条自动尺寸同步在 QA 结果展示期不能继续自由 shrink；前端会把 `useAutoResizeWindow` 切到“只增不减”的受限模式，让后续 Markdown / 高亮 / Mermaid 等异步内容仍能把窗口撑开，同时避免短时测量回退把 `NSPanel.set_content_size` 又缩回去截断内容
- `resize_main_window(...)` 的调用时间必须保持可观测；排查 macOS 失焦问题时，日志里应该能直接看到每次原生 resize 的请求尺寸与实际尺寸，不能把关键窗口事件只埋在 `debug` 级别
- 纯 OCR 回填不再复用泛化的 `selected-text` 事件；窗口层会单独发 `ocr-captured-text`，把 `sourceMode=ocr` 和识别文本一起带给前端，避免输入来源语义丢失
- 快捷翻译窗口事件拆成“开始”和“结果”两段：拿到原文后窗口层先发 `ocr-translation-started` 并立即显示 launcher，等后台翻译完成后再发 `ocr-translation-result`；结果回填不应再次依赖首次显示窗口来驱动 UI
