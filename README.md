# Wabity

`Wabity` 是一个基于 `Tauri v2 + React + TypeScript + Rust` 的桌面 launcher。

当前仓库已完成第一批可运行骨架：

- `Tauri v2` 桌面壳与 `React` 前端集成
- 全局快捷键切换主窗口
- macOS 下支持全局快捷键触发交互式截图 OCR；`Alt+R` 只识别后回填 launcher，`Alt+D` 会先尝试翻译当前应用的选中文本，只有没有选中文本时才回退到截图 OCR 并翻译
- OCR provider 现已支持本地 macOS Vision 和远程 OpenAI 兼容多模态模型
- 设置页已把快捷键、外观和 OCR 配置并入“通用”；“AI 功能”页按“翻译配置 / 文档问答配置”两个任务卡片维护各自的模型和系统提示词；LLM 页面统一维护 OpenAI 风格接入点下的单模型条目：配置类型直接区分 `LLM · responses stateless`、`LLM · responses stateful`、`LLM · chat/completions` 和 `Embedding`，其中 `responses` 页面仍可额外声明多模态；RAG 配置页支持用这些 embedding 模型为选中目录中的 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc` 文件构建并持续维护本地 LanceDB 向量索引，配套 SQLite 元数据缓存、watcher 增量维护、`staged/active` 版本切换，以及基于 `原文文本 + embedding fingerprint` 的全局向量复用以减少重复 embedding
- 透明窗口 + 圆角 launcher 外观
- 默认单行输入框，可按 `Cmd/Ctrl+Enter` 切到多行模式
- 普通动作补全不再对任意非空输入立刻弹出；只有显式 `/` 命令、`http`/`{` 这类强信号输入，或满足 2 个英文字符 / 1 个非英文字符后才显示候选
- `/` 候选只保留真实可执行命令，并以“主命令 + 简短说明”展示，不暴露内部匹配评分
- JSON 格式化命令以 `/format` 为主，支持 `/fmt` 与兼容别名 `/json`；输入合法 JSON 载荷时会在输入框下方直接显示 pretty format 预览
- `/base64` 会对载荷自动判别：合法 Base64 文本优先解码，否则按普通文本编码为 Base64
- 光标所在 `@token` 会触发当前 workspace 的模糊文件搜索；查询满足 2 个英文字符或 1 个非英文字符后，在输入后 50ms 发起查询
- 文本类动作执行与结果反馈
- OCR provider 抽象、macOS Vision 实现，以及远程 LLMOCR 实现
- 当前 workspace 写入 `config.toml`，最近目录历史单独写入 `workspace-history.toml`
- 设置页已把 `ACP Agent` 和 `MCP` 拆成两个一级菜单：ACP Agent 页以编辑表单为主，预设改成下拉填表入口；MCP server 改成全局共享清单，并在建会话时通过 ACP `mcp_servers` 统一透传给选中的 agent

## 开发命令

```bash
npm install
npm run clean:target -- --dry-run
npm run format
npm run lint
npm run tauri dev
```

如果 `src-tauri/target` 积累了大量旧构建产物，可用下面的命令按修改时间清理：

```bash
npm run clean:target -- --days 3
```

默认只删除超过 3 天未更新的常见 Rust/Tauri 构建产物目录与顶层二进制；先预览可加 `--dry-run`，需要全清已识别构建产物可用 `--all`。

Rust 侧当前依赖系统 `protoc`。在 macOS + Homebrew 环境下，确保 `protoc` 已安装并可执行，例如 `/opt/homebrew/bin/protoc`。

## 打包命令

macOS 下显式构建 `dmg`：

```bash
npm run tauri:build:macos:dmg
```

产物输出到 `src-tauri/target/release/bundle/dmg/`，文件名会跟随 `productName`，例如 `Wabity.dmg`、`Wabity.exe`。

当前把 macOS 的 bundle 目标单独放在 `src-tauri/tauri.macos.conf.json`，避免污染其他平台的默认打包目标；Tauri 会在 macOS 构建时自动合并这份平台配置。

当前原生启动阶段的异步初始化统一挂到 `tauri::async_runtime`，不要在 `setup` 中直接依赖 `tokio::runtime::Handle::current()`；那会在 Tauri 尚未进入 Tokio 上下文时直接 panic。

## 当前范围

本次实现仍然只覆盖 `IMPLEMENTATION_PLAN.md` 中的 `M1/M2` 主链路，并额外补了：

- 极简 launcher UI
- 透明圆角窗口退化方案
- 用户目录模糊文件搜索

还没有完成 OCR 历史、非 macOS 交互式截图、历史排序和完整系统动作接入。
