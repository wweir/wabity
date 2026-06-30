# Product

## Register

product

## Users

Wabity 面向懂一点技术但不以 IDE 为主工作台的桌面用户。用户通常在 macOS 桌面环境中用全局快捷键唤起 launcher，快速启动应用、执行轻量命令、搜索当前 workspace 文件、查看工作区相关结果、维护剪贴板文本，以及配置 AI / OCR / RAG / Agent 能力。

## Product Purpose

Wabity 是一个 Tauri + React + Rust 桌面 launcher，目标是把“输入到动作”的本地工作流压缩到一个低干扰入口中。它不是插件市场、远程 agent 编排平台或 IDE 替代品；成功标准是用户能稳定、快速、可理解地完成桌面动作、轻量问答、文档检索、翻译/OCR 和内嵌 Agent session 管理。

## Brand Personality

冷静、专业、平和。界面应传达稳定、可信、低压迫感，避免过度装饰、炫技感和泛化 AI 工具模板味。

## Anti-references

- 泛滥的 frosted glass / glow / 紫蓝渐变 AI 工具模板。
- 过于温吞的米白、砂纸、奶油色默认生成式界面。
- 视觉层级扁平、像通用生成器产物的 pill 网格。
- 未接通的伪能力入口，尤其是外部 ACP agent、远程 agent transport 编排或插件市场暗示。
- 把轻量 RAG 问答和长期 Agent session 混成同一种会话模型。

## Design Principles

1. 核心操作必须一眼可懂，不能靠 placeholder 和弱提示词撑交互。
2. 视觉层级靠结构、排版和对比建立，不靠大面积玻璃感和浅色描边伪装精致。
3. 所有界面共享稳定 token、尺寸和状态语义，launcher 与 settings 不得割裂。
4. 桌面端优先键鼠和键盘可达性，同时保证窄窗口下不崩。
5. 设置项必须所见即所得；停用或迁移中的能力必须明确降级，而不是保留陈旧入口。

## Accessibility & Inclusion

目标至少满足 WCAG AA 的可读性和键盘可达性基线。正文与控件文本需要保持足够对比度；快捷键、弹层、session 切换、设置导航和错误提示必须可通过键盘理解和操作。动画和视觉反馈应尊重 reduced motion，不能依赖动效才能传达状态。
