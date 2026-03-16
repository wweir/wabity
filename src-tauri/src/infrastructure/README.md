# infrastructure

职责：

- `window`：管理 launcher 主窗口的显示、隐藏、聚焦与平台窗口行为
- `hotkey`：解析、注册、注销全局快捷键
- `config`：集中处理本地配置、workspace 历史读写与缓存

关键约束：

- `window` 只负责窗口壳层行为，不承载业务状态或动作匹配逻辑
- `config` 统一将用户配置写入用户配置目录下的 `wabity/config.toml`，其中包含 ACP agent 与全局 MCP 清单；workspace 最近目录历史写入 `wabity/workspace-history.toml`
- `config` 负责 TOML 序列化、原子 `safe_write`、磁盘读写和内存缓存；启动时由 `AppState::new` 先读取，再把配置投影到运行时状态
- 配置模型新增字段时必须保持向后兼容；旧版 `config.toml` 缺字段时应通过 `serde(default)` 回填，而不是在启动阶段直接解析失败
- macOS 下 launcher 启动时会把主窗口转换成 `NSPanel`，并配置为 `nonactivating panel`；再叠加 `can_join_all_spaces + full_screen_auxiliary + stationary` 和高层级，这是覆盖全屏 Space 的主实现，不再把普通 `NSWindow` 的补丁当成可靠方案
- launcher 每次显示前都会优先按鼠标当前所在显示器选择目标屏幕，再按该显示器 `work_area` 计算默认位置，而不是依赖配置里的原生 `center`：水平居中，垂直中心落在可用高度 `0.382` 处
- 前端首轮自动测量会继续回写真实窗口尺寸；`window` 模块只会在 launcher 第一次真正显示前跟随这些 resize 重新套用默认位置，避免初始宽度变化导致左右不居中，同时不覆盖用户后续手动拖动的位置
- 运行时平台特性配置必须在主线程执行，避免直接从普通线程调用 AppKit
- macOS 下命中 `NSPanel` 的显隐、置前和尺寸调整统一通过 Tauri `run_on_main_thread` 调度；后台 OCR 任务结束后也只能经这条通道回到窗口层
- launcher 显隐不依赖底层 `is_visible()` 查询，而是维护独立运行时状态；全局快捷键切换按“按下一次只触发一次，直到收到 Released 才重新解锁”的状态机处理，避免同一轮组合键被重复 toggle
