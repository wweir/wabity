# Wabity

`Wabity` 是一个基于 `Tauri v2 + React + TypeScript + Rust` 的桌面 launcher。

当前仓库已完成第一批可运行骨架：

- `Tauri v2` 桌面壳与 `React` 前端集成
- 同一用户登录会话内只允许一个 launcher 原生实例；重复启动会直接唤醒已有窗口
- 全局快捷键支持切换主窗口、翻译选中文本/OCR，以及直接打开历史剪贴板；默认分别为 `Alt+Space`、`Alt+D`、`Alt+V`
- macOS 下支持全局快捷键触发交互式截图 OCR；`Alt+D` 会先尝试翻译当前应用的选中文本，只有没有选中文本时才回退到截图 OCR 并翻译
- OCR provider 现已支持本地 macOS Vision 和远程 OpenAI 兼容多模态模型
- 设置页已把快捷键、外观和 OCR 配置并入“通用”；“AI 功能”页按“翻译配置 / 文档问答配置”两个任务卡片维护各自的模型和系统提示词；LLM 页面统一维护 OpenAI 风格接入点下的单模型条目：配置类型直接区分 `LLM · responses stateless`、`LLM · responses stateful`、`LLM · chat/completions` 和 `Embedding`，其中 `responses` 页面仍可额外声明多模态，并提供 `OpenAI / DeepSeek / Ollama / 智谱 / SiliconFlow` 等内置模板；RAG 配置页支持用这些 embedding 模型为选中目录中的 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc`、`.docx`、`.pdf` 文件构建并持续维护本地文档索引，底层采用 SQLite 元数据与 chunk 真相源配合 USearch 派生向量索引，支持 watcher 增量维护、`staged/active` 版本切换，以及基于 `原文文本 + embedding fingerprint` 的全局向量复用以减少重复 embedding；当前按文档类型限制单文件大小：纯文本/Markdown 20 MB、`docx` 16 MB、`pdf` 8 MB
- 透明窗口 + 圆角 launcher 外观
- 默认单行输入框，可按 `Cmd/Ctrl+Enter` 插入换行并切到多行模式；单行和多行都用 `Enter` 执行
- 历史剪贴板通过全局快捷键 `Alt+V` 打开独立面板：后台只保留少量文本记录，并支持固定少量常用项；如果 launcher 已在前台，选中某条会插入 launcher 输入框；否则会写回系统剪贴板、记住呼出前的前台应用、隐藏 launcher、重新激活原应用，并在确认目标应用重新成为前台后再发送粘贴快捷键
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

`npm run tauri dev` 现在会自动走 `scripts/tauri-cli.sh` 包装层，开发态默认仍写入 `src-tauri/target`，并在启动前自动清理一次超过 6 小时未更新的 `src-tauri/target/debug/deps` 和 `src-tauri/target/debug/incremental`。若确实需要改到别的位置，可在命令前显式设置 `WABITY_TAURI_DEV_TARGET_DIR`。

```bash
WABITY_TAURI_DEV_TARGET_DIR="$HOME/.cache/wabity-tauri-dev-target" npm run tauri dev
```

如果 `src-tauri/target` 已经积累了大量旧构建产物，可用下面的命令按修改时间清理：

```bash
npm run clean:target -- --days 3
```

默认只删除超过 3 天未更新的常见 Rust/Tauri 构建产物目录与顶层二进制；先预览可加 `--dry-run`，需要全清已识别构建产物可用 `--all`。

如果只想清理开发期最占空间的 `src-tauri/target/debug/deps` 和 `src-tauri/target/debug/incremental`，使用：

```bash
npm run clean:target:debug-cache -- --days 3
```

默认就是按 6 小时阈值处理；等价于：

```bash
bash ./scripts/clean-target.sh --scope debug-cache --hours 6
```

这个模式只会检查上述两个目录本身的更新时间；超过指定小时数未更新时，整目录删除。先预览可加 `--dry-run`，需要改阈值可显式传 `--hours <n>`，需要全清这两类缓存可加 `--all`。

Rust 侧当前依赖系统 `protoc`。在 macOS + Homebrew 环境下，确保 `protoc` 已安装并可执行，例如 `/opt/homebrew/bin/protoc`。

## 打包命令

macOS 下显式构建 `dmg`：

```bash
npm run tauri:build:macos:dmg
```

这个脚本会先启动一个本地补丁 watcher，在 `@tauri-apps/cli` 生成临时 `bundle_dmg.sh` 后立即打补丁：保留 Finder AppleScript 美化流程，但给 `osascript` 增加重试和最终降级容错，避免因为 Finder 自动化权限、前台会话时序或偶发 `-1728` 之类错误直接导致整个 DMG 构建失败。同时会把 Tauri 默认写入的 `.VolumeIcon.icns` 延后到 Finder 布局之后再复制，避免隐藏卷图标文件参与根目录排版导致图标错位。

macOS 专属配置 `src-tauri/tauri.macos.conf.json` 现在还会固定 DMG 的窗口尺寸、窗口初始位置，以及 `Wabity.app` / `Applications` 的主安装动线坐标；本地补丁则继续负责注入背景图、兜底修复脚本的坐标、隐藏扩展名，以及更适合展示的图标/文字尺寸，避免 Finder 自动排版把安装入口和辅助入口挤乱。

生成出来的 DMG 现在还会额外包含一个用户可见辅助文件：

- `修复.command`：把 `Wabity.app` 复制到 `/Applications`，尝试清理隔离属性，并自动启动应用

产物输出到 `src-tauri/target/release/bundle/dmg/`，文件名会跟随 `productName`，例如 `Wabity.dmg`、`Wabity.exe`。

## 首次打开

当前发布包还没有 Apple Developer ID 签名和 notarization，因此普通下载链路下，macOS 仍可能在首次打开时要求人工确认。

推荐顺序：

1. 把 `Wabity.app` 拖到 `Applications`
2. 在 `Applications` 中对 `Wabity.app` 右键，选择“打开”
3. 如果系统仍拦截，回到 DMG 后双击 `修复.command`
4. 如果还有拦截，去“系统设置 -> 隐私与安全性”里选择“仍要打开”

当前把 macOS 的 bundle 目标单独放在 `src-tauri/tauri.macos.conf.json`，避免污染其他平台的默认打包目标；Tauri 会在 macOS 构建时自动合并这份平台配置。

## GitHub Release

仓库现在包含 tag 驱动的 GitHub Actions 工作流 [`.github/workflows/release-macos-dmg.yml`](/Users/wweir/Sites/Mine/wabity/.github/workflows/release-macos-dmg.yml)。

推送形如 `v0.1.0` 的 tag 时，工作流会在 `macos-latest` 上执行下面的固定流程：

1. 校验 tag 版本是否和 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 一致
2. 执行现有的 `npm run tauri:build:macos:dmg`
3. 把生成的 `src-tauri/target/release/bundle/dmg/*.dmg` 上传到对应的 GitHub Release

示例：

```bash
git tag v0.1.0
git push origin v0.1.0
```

这个工作流只负责生成并上传未签名的 `dmg`。如果后续要分发给普通 macOS 用户并降低系统拦截，还需要额外补代码签名和 notarization；那是另一条发布约束，不能和“先把 DMG 自动挂到 Release”混为一谈。

另外，普通分支推送会触发单独的校验工作流 [`.github/workflows/build-debug.yml`](/Users/wweir/Sites/Mine/wabity/.github/workflows/build-debug.yml)。它只做一次 macOS debug 编译校验：安装依赖、构建前端，并执行 `tauri build --debug --no-bundle`，目标是尽早发现“代码已经不能编译”的问题，而不是顺便发版。

当前原生启动阶段的异步初始化统一挂到 `tauri::async_runtime`，不要在 `setup` 中直接依赖 `tokio::runtime::Handle::current()`；那会在 Tauri 尚未进入 Tokio 上下文时直接 panic。

## 当前范围

本次实现仍然只覆盖 `IMPLEMENTATION_PLAN.md` 中的 `M1/M2` 主链路，并额外补了：

- 极简 launcher UI
- 透明圆角窗口退化方案
- 用户目录模糊文件搜索

还没有完成 OCR 历史、非 macOS 交互式截图、历史排序和完整系统动作接入。
