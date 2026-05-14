# Screenshot Review 与 ScreenCaptureKit 截图链路设计（2026-05-14）

## 1. 目标与非目标

目标：把 `Alt+D` 的无选中文本路径收敛为可确认的 Screenshot Review 工作流，并把截图采集主路径切到 macOS `ScreenCaptureKit`。

稳定入口语义：

1. 有选中文本：继续直接翻译当前应用选中文本，不进入 Screenshot Review。
2. 无选中文本：打开 Wabity 透明 overlay 让用户拖拽选择区域。
3. Rust 收到区域后调用 `ScreenCaptureKit` 拿到截图图像，再用本地 ImageIO 编码 PNG，再由 Rust 落盘。
4. OCR 识别截图，打开 Review UI。
5. 用户确认、编辑或选择 OCR 块后，再翻译或复制文本。

非目标：

- 不做高级 OCR、屏幕结构解析、自动点击或窗口语义理解。
- 不做图片 + prompt 的通用多模态问答入口。
- 不做跨平台截图采集；当前交互式截图仍仅支持 macOS。
- 不保留 shell `screencapture` fallback。ScreenCaptureKit 失败应显式报错。

## 2. 分层职责

### `infrastructure/screen_capture`

只封装平台截图能力：

- 创建 `screenshot-capture-overlay` 透明窗口。
- 管理 overlay region selection token，防止过期 IPC 误提交。
- 把 overlay logical rect 转成全局 screen-space points。
- 调用 `ScreenCaptureKit` 获取区域截图图像，再用本地 ImageIO 编码 PNG，再由 Rust 写入 Wabity 专用临时目录。
- 返回截图路径、backend、capture mode 和 rect metadata。
- 删除截图临时文件。

它不执行 OCR、不读取 LLM 设置、不决定翻译行为。

### `services/screenshot_review`

编排用例：

- 接收 `ScreenCaptureResult`。
- 调用当前 OCR provider 识别截图。
- 创建 review session。
- 输出 preview URL、截图尺寸、capture metadata、OCR 状态和文本块。
- 在确认、取消、重试时清理 session 与临时截图文件。

OCR 失败不是截图流程 fatal error；Review UI 仍应展示错误，让用户重试或手动编辑文本。

### `features/launcher`

负责用户确认：

- `ScreenshotReviewPanel` 展示截图、OCR 状态、块选择和可编辑文本。
- 最终提交文本必须来自用户确认的 textarea / selected blocks / OCR text fallback。
- `retry` 关闭当前 session 后重新进入截图链路。
- `cancel` 只结束 session，不触发翻译。

### `app/ScreenCaptureOverlay`

独立 window UI：

- 只处理拖拽矩形、Escape 取消和最小尺寸校验。
- 通过 IPC 提交 `complete_screen_capture_region(token, rect)` 或 `cancel_screen_capture_region(token)`。
- 不加载 launcher/settings，不做 OCR 和翻译。

## 3. 数据流

```text
Alt+D
  -> selection::read_selected_text
  -> 有文本：execute_shortcut_translation(selection)
  -> 无文本：hide visible shortcut windows
      -> screen_capture::capture_user_selected_region
          -> open overlay
          -> frontend complete/cancel region IPC
          -> ScreenCaptureKit returns image
          -> ImageIO encodes PNG and Rust writes the file
      -> screenshot_review::create_session_from_capture
          -> OCR provider recognize
          -> store review session
      -> window::show_main_window_with_screenshot_review
      -> user confirm/copy/cancel/retry
```

Payload 形态：

```ts
interface ScreenshotReviewPayload {
	sessionId: string;
	previewUrl: string;
	imageWidth: number;
	imageHeight: number;
	capture: {
		backend: "screen_capture_kit";
		mode: "region";
		rect?: { x: number; y: number; width: number; height: number } | null;
	};
	ocr: {
		provider: "system" | "llm_ocr";
		status: "success" | "empty" | "failed";
		text: string;
		errorMessage?: string | null;
		blocks: ScreenshotReviewBlock[];
	};
	requestedAction: "translate";
}
```

## 4. ScreenCaptureKit 坐标约束

- overlay 覆盖当前鼠标所在 monitor。
- 前端提交的是 overlay client logical px。
- Rust 创建 overlay 时记录 monitor logical origin 和 size。
- 全局 capture rect = monitor logical origin + selected client rect。
- `ScreenCaptureKit` region API 接收 screen-space points；这里使用 logical point rect。

取消语义：

- Escape 或用户不提交有效区域：返回 `Ok(None)`，恢复此前隐藏的快捷窗口。
- token 不存在或 rect 非法：返回错误，不吞掉。
- ScreenCaptureKit 没有返回可写入的截图图像，或 ImageIO 没有写出空文件：返回错误。

## 5. 权限与隐私

- `ScreenCaptureKit` 依赖 macOS Screen Recording 权限；权限失败必须显示为截图失败，不 fallback 到旧 shell 命令。
- 系统 OCR 在本机处理截图。
- LLM OCR 会在 Review 出现前把截图发送给已配置的多模态 OCR 模型，因为 Review 依赖 OCR 结果；设置页和 Review UI 必须提示这一点。

## 6. 依赖决策

新增 macOS-only Apple framework binding：

- `objc2-screen-capture-kit`：调用 `SCScreenshotManager`。
- `objc2-core-foundation`：构造 `CGRect` / `CGPoint` / `CGSize`。
- `objc2-uniform-type-identifiers`：指定 PNG content type。
- `block2`：桥接 ScreenCaptureKit completion handler block。
- `objc2-image-io`：把 `CGImage` 编码成 PNG 数据，再由 Rust 明确落盘，避免依赖 `SCScreenshotConfiguration.fileURL` 的写文件行为。

这些依赖是 Apple framework Rust binding，不是第三方截图 SDK；范围限定在 `target_os = "macos"`。

## 7. 实施阶段

- Phase 1：文档与边界确认。
- Phase 2：Screenshot Review + ScreenCaptureKit region backend。
- Phase 3：按需补 window/display capture 入口。
- Phase 4：按需设计图片 + prompt 多模态入口。
- Phase 5：按需评估高级 OCR / screen parser。

当前落地到 Phase 2；Phase 3 之后都不是本次范围。

### 2026-05-14 清理记录

- 旧 shell `screencapture` 交互截图路径已删除；当前唯一截图 backend 是 ScreenCaptureKit。
- 旧 `ocr_capture` 快捷键配置不再作为兼容字段接受；快捷键配置只保留 `toggle_launcher`、`ocr_translate`、`open_clipboard_history`。
- launcher 错误展示统一走 `launcher-failure` 事件；Screenshot Review 失败只是其中一种消息，不再复用截图专用失败事件承载普通翻译失败。
