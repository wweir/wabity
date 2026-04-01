# RAG Document Ingestion Design

## 背景

当前仓库的 RAG 索引链路只支持纯文本文件：

- 支持的后缀仅包含 `md`、`mdx`、`txt`、`markdown`、`rst`、`adoc`
- 扫描阶段直接读取原文件字节并要求内容是合法 UTF-8 文本
- chunk 元数据当前以 `line_start`、`line_end`、`paragraph_line_start`、`heading_path` 为核心定位信息
- 问答阶段通过内置工具 `wabity.read_file_lines` 回读原始文本文件，补足精确证据

这套实现对纯文本成立，对 `doc`、`docx`、`pdf` 不成立。根因不是“扩展名没放开”，而是现有链路把“原文件就是 UTF-8 文本”当成了前提。

如果继续沿用这个前提，会出现三类问题：

1. 扫描阶段无法直接读取二进制文档
2. 检索阶段缺少稳定的页码、段落、标题锚点
3. 问答阶段即使检索命中，也无法继续通过 `wabity.read_file_lines` 精读原文件

## 目标

在不推翻现有 LanceDB、SQLite 元数据、chunk 复用、embedding 缓存和问答工具循环架构的前提下，让 RAG 索引支持：

- `docx`
- `pdf`
- `doc`

设计目标是：

- 最小侵入复用现有索引主链路
- 不把文件格式解析逻辑继续堆进 `rag.rs`
- 保持检索结果仍可被问答链路追溯和精读
- 明确不同格式的能力边界，不用一个抽象掩盖真实差异

## 当前状态

- 2026-03-26：第一步已落地 `docx`
- 2026-03-28：第二步已落地文本型 `pdf`
- 2026-03-30：PDF 抽取实现从 `pdf-extract` 切到 `lopdf`，原因是前者对畸形 content stream 存在进程内 panic 风险
- 当前实现新增独立 `document_extract` 模块
- `docx` 会先解 ZIP，读取 `word/document.xml`，并在可用时读取 `word/styles.xml`
- 抽取结果会被规范化成 Markdown 风格文本，再复用现有标题路径和语义切块逻辑
- 文本型 `pdf` 会先使用 `lopdf` 按页抽取文本、做轻量页眉页脚去噪，并切成页级 block；单页解析失败只降级为 warning；当前 chunk 主锚点为 `page_start/page_end`
- `wabity.read_document_excerpt` 已支持按 `path + chunk_index` 回读抽取型文档摘录，`wabity.read_file_lines` 继续只服务文本行语义稳定的文档
- 旧二进制 `.doc` 仍未实现

## 非目标

- 不在第一阶段支持扫描版 PDF OCR
- 不在第一阶段做任意 Office 格式全集兼容
- 不为 `doc` 自研完整二进制解析器
- 不把所有文档统一降级成“只有一大段文本、没有结构锚点”的最低形态
- 不为了支持二进制文档而破坏现有文本文件的 `line_start/line_end` 语义

## 现状约束

当前设计有两个关键约束，必须正视：

### 1. 索引链路假设输入是文本

`rag` 当前会在扫描阶段直接：

- 判断后缀是否在纯文本白名单中
- 读取文件字节
- 拒绝包含 NUL 的内容
- 把字节按 UTF-8 文本解析

这意味着 `doc/docx/pdf` 不能通过“扩后缀白名单”接入。那种做法不是方案，只是把错误从“跳过文件”改成“运行时报错”。

### 2. 问答链路假设证据可按文本行回读

`rag_answer` 当前通过 `wabity.read_file_lines` 读取原文件文本，再把读取结果回填给模型。

这意味着：

- 对 `pdf`，原文件没有稳定“文本行”语义
- 对 `docx`，标题、段落、列表和表格在原始 XML 结构里，不是天然按行文本
- 对 `doc`，原文件更不是可直接按 UTF-8 行读取的文本

如果只补索引、不补问答精读工具，最终会得到一个半残能力：能搜到命中，但模型无法继续验证原文细节。

## 总体方案

推荐新增一层独立的“文档抽取层”，放在索引入口之前。

核心链路改为：

`Path -> DocumentExtractor -> ExtractedDocument -> chunk -> embedding -> LanceDB`

而不是：

`Path -> 直接读 UTF-8 文本 -> chunk -> embedding -> LanceDB`

### 分层建议

新增模块：

- `src-tauri/src/services/document_extract/mod.rs`
- `src-tauri/src/services/document_extract/docx.rs`
- `src-tauri/src/services/document_extract/pdf.rs`
- `src-tauri/src/services/document_extract/doc.rs`

职责边界：

- `document_extract`
  - 负责文件格式识别
  - 负责把原文档抽取成规范化文本和结构块
  - 负责返回抽取警告、页码、标题路径、段落锚点等格式相关元数据
- `rag`
  - 继续负责扫描、分块、chunk 元数据归并、embedding、缓存、版本切换和向量写入
- `rag_answer`
  - 继续负责问答工具循环
  - 但需要补一个基于规范化文档内容的精读工具，而不是只读原文件行

这条边界符合当前仓库分层。否则 `rag.rs` 会继续膨胀成“扫描器 + 分块器 + 文件格式解析器 + 数据库写入器 + 兼容层杂物箱”。

## 核心数据模型

抽取层不要只返回一个 `String`。那样会把后续引用、问答精读和错误诊断全部做烂。

建议新增：

```rust
pub enum DocumentKind {
    PlainText,
    Markdown,
    Pdf,
    Docx,
    Doc,
}

pub struct ExtractedDocument {
    pub kind: DocumentKind,
    pub absolute_path: String,
    pub extractor_fingerprint: String,
    pub normalized_text: String,
    pub blocks: Vec<ExtractedBlock>,
    pub warnings: Vec<String>,
}

pub struct ExtractedBlock {
    pub text: String,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub heading_path: Vec<String>,
    pub anchor_label: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
}
```

几个约束：

- `line_start/line_end` 改成可选，而不是假装所有格式都有稳定行号
- PDF 至少要有页码
- DOCX 至少要有标题路径或段落锚点
- `extractor_fingerprint` 必须参与文件级缓存判定，否则以后切换抽取器实现时会错误复用旧索引

## 索引链路改造

### 1. 文件类型识别

推荐把当前“仅按文本扩展名判断是否支持”改成更清晰的检测：

- `md/mdx/markdown` -> `Markdown`
- `txt/rst/adoc` -> `PlainText`
- `pdf` -> `Pdf`
- `docx` -> `Docx`
- `doc` -> `Doc`

这里仍可先用扩展名做一级路由。没有必要在第一版引入魔数识别，但接口应允许后续扩展。

### 2. 抽取后再切块

对于纯文本和 Markdown：

- 基本沿用现有逻辑
- 只是把输入来源从“直接读文件文本”改成“抽取器返回的规范化文本”

对于 PDF 和 Office：

- 不建议直接把整篇文档扁平成一大段文本再走通用 `TextSplitter`
- 应优先使用抽取层提供的 `blocks`
- 再由 `rag` 在 block 边界上打包成 chunk

这样做的原因不是形式主义，而是要保留最基本的结构定位能力。

### 3. 元数据扩展

LanceDB chunk 行和查询返回结构建议补充：

- `document_kind`
- `page_start`
- `page_end`
- `anchor_label`

保留现有字段：

- `line_start`
- `line_end`
- `paragraph_line_start`
- `heading_path`

但需要调整语义：

- 文本文件：继续保持强语义
- PDF / DOC / DOCX：允许 `line_*` 为空，`heading_path` 或 `page_*` 承担主锚点

### 4. 缓存与失效

当前文件级缓存基于：

- `size`
- `mtime`
- `content_md5`
- `embedding_fingerprint`

接入抽取层后，应把 `extractor_fingerprint` 一并纳入重建目标。

否则会出现一个明显错误：

- 原文件没变
- embedding 模型没变
- 但抽取器升级了，文本归一化结果变了
- 系统却错误复用旧 chunk

这是缓存污染，不是优化。

## 问答链路改造

### 1. 不再只依赖 `wabity.read_file_lines`

推荐保留：

- `wabity.read_file_lines`

新增：

- `wabity.read_document_excerpt`

职责区分：

- `read_file_lines`
  - 只服务真实文本文件
  - 保持当前白名单和安全边界
- `read_document_excerpt`
  - 服务 PDF / DOC / DOCX
  - 读取索引时持久化的规范化文本片段或文档摘录缓存
  - 支持按 `page_start/page_end`、`anchor_label`、`heading_path` 或 chunk 位置精读

这比强迫模型继续调用“读文件行”合理得多。否则工具名和真实能力会长期错位。

### 2. citation 语义调整

当前 citation 主要依赖：

- `path`
- `absolutePath`
- `lineStart`
- `lineEnd`
- `paragraphLineStart`
- `headingPath`

接入文档格式后，citation 需要允许：

- `pageStart`
- `pageEnd`
- `anchorLabel`

并且前端展示策略应改成：

- 有页码时优先显示页码
- 有标题路径时显示标题路径
- 只有文本文件时再突出行号

如果继续把 PDF 命中包装成伪造行号，只会让引用看起来精确，实际不可验证。

## 分格式方案

### DOCX

这是最适合优先落地的格式。

原因：

- 本质是 ZIP + XML，结构相对规整
- 可稳定抽取标题、段落、列表、表格
- 很容易归一化成适合现有语义切块逻辑的文本

当前实现：

- 依赖 `zip`
- 依赖 `roxmltree`
- 在本地读取 `document.xml` / 可选 `styles.xml`
- 把标题、段落、列表和表格规范化为 Markdown 风格文本

推荐继续保持这个方向，不引入外部 Office 进程。

抽取内容：

- `word/document.xml`
- 视需要读取 `word/styles.xml`
- 视需要读取 `word/numbering.xml`

归一化策略：

- 标题样式映射到 `heading_path`
- 普通段落转文本段
- 列表项转为 `- item`
- 表格转为 `cell1 | cell2 | cell3`
- 连续空白折叠

不推荐把 DOCX 读取外包给过重的外部进程。这个格式完全值得在当前仓库里做成本地轻量实现。

### PDF

第一阶段只支持“可直接提取文本的 PDF”，不支持扫描版 OCR。

推荐实现：

- 使用 `lopdf` 直接按页提取文本，避免引入会在坏 PDF 上 panic 的抽取层

抽取策略：

- 先按页抽取
- 页内再按空行或明显段落边界切成 block
- chunk 元数据保留 `page_start/page_end`

需要明确的风险：

- PDF 的阅读顺序不天然可靠
- 多栏布局、页眉页脚、脚注可能污染正文
- 中文和复杂排版质量取决于抽取库表现

因此 PDF 接入必须带“抽取质量可能退化”的显式警告能力，不能把抽取结果伪装成和 Markdown 同等可靠。

### DOC

这是成本最高、收益最低的一类。

不推荐在 Rust 内部自研 `.doc` 二进制解析。

推荐方案：

- 先实现“外部转换器适配层”
- 启动时探测系统里是否存在受支持的转换器
- 若存在，则调用并读取标准输出文本
- 若不存在，则明确报错并提示用户先转为 `docx`

原因：

- `.doc` 是旧二进制格式，维护成本远高于 `docx`
- 解析正确率、编码兼容、表格和样式恢复都很差
- 这不是当前产品的核心竞争力

对 `.doc` 的策略应当是“尽可能支持”，不是“承诺高质量原生解析”。

## 依赖建议

### 已引入

- `zip`
  - 用于读取 `docx`
  - 标准、轻量、问题边界明确
- `roxmltree`
  - 用于解析 `docx` XML
  - 当前需求只需要只读遍历和最小结构提取，用树模型实现更直接

### 后续可能引入

- 更强的 PDF 专用抽取器
  - 仅当 `lopdf` 的文本顺序或字体兼容性无法满足实际文档质量时再评估
  - 前提是库必须先证明自己不会把坏输入升级成线程 panic

### 暂不建议引入

- LibreOffice / Pandoc 作为统一转换层
  - 依赖重
  - 启动慢
  - 跨平台分发和故障诊断都更麻烦
- 为 `.doc` 引入复杂原生解析 crate
  - 成本不成比例
- 直接把 OCR 链路并入 PDF v1
  - 这是另一条能力线，不应污染本次落地范围

## 实施阶段

### 阶段 1：抽取层骨架

- 新增 `document_extract` 模块
- 定义 `DocumentKind`、`ExtractedDocument`、`ExtractedBlock`
- 把 `rag` 扫描入口改成“先抽取，再切块”
- 纯文本和 Markdown 行为保持不变

交付标准：

- 不影响现有纯文本索引
- 现有测试大体无需重写

### 阶段 2：DOCX 接入

- 用 `zip + quick-xml` 接入 `docx`
- 支持标题、段落、列表、表格的基础抽取
- 让 chunk 元数据能保留 `heading_path`

交付标准：

- `docx` 文档能被扫描、分块、检索
- 引用可显示标题路径

### 阶段 3：PDF 接入

- 用 `lopdf` 接入文本型 PDF
- 抽取结果按页组织
- citation 支持页码

交付标准：

- 文本型 PDF 能被扫描、分块、检索
- 检索结果能显示页码锚点

阶段状态：

- 已完成
- 当前仍未支持扫描版 PDF OCR

### 阶段 4：问答精读补齐

- 新增 `wabity.read_document_excerpt`
- `rag_answer` 根据 citation 类型选择精读工具
- 前端 citation 展示兼容页码和段落锚点

交付标准：

- 模型可以继续精读 PDF / DOCX 的规范化片段
- 不再依赖伪造“行号”完成验证

阶段状态：

- 已完成 `wabity.read_document_excerpt` 和前端 citation 页码展示
- `docx` 当前仍兼容 `wabity.read_file_lines`；后续若要彻底统一抽取型文档精读入口，再收口到同一个 excerpt 工具

### 阶段 5：DOC 适配

- 增加外部转换器探测
- 增加 `.doc` 的 best-effort 文本抽取
- 缺失转换器时返回明确错误

交付标准：

- `.doc` 支持是显式降级能力，不影响主链路稳定性

## 测试策略

### 单元测试

优先覆盖：

- 文件类型识别
- DOCX 标题/段落/列表/表格抽取
- PDF 页级 block 组织
- 抽取器 fingerprint 变化触发重建
- 非文本格式不会再误走 `read_file_lines`

### 集成测试

至少补一组：

- `docx -> build index -> query -> citation`
- `pdf -> build index -> query -> citation`
- `read_document_excerpt` 能读取规范化片段

### 回归测试

必须确保：

- 现有纯文本和 Markdown 索引行为不变
- 现有 metadata cache 和 chunk reuse 语义不被破坏
- 现有 `rag_answer` 文本文件精读能力不回退

## 风险与取舍

### 1. PDF 抽取质量不可完全保证

这不是实现粗糙，而是 PDF 这种格式本身决定的。

所以产品语义必须承认：

- PDF 检索可支持
- 但抽取可靠性低于 Markdown / TXT / DOCX

### 2. DOC 不值得追求“原生完美支持”

如果把 `.doc` 和 `.docx` 视为同等级能力，就是错误判断。

合理策略是：

- `docx` 做好
- `pdf` 做到可用
- `doc` 做明确降级

### 3. 规范化文本会引入“与原始版面不完全一致”的偏差

这是必要代价。

RAG 真正需要的是：

- 稳定可检索文本
- 可被模型继续读取的局部上下文
- 可供用户理解的大致定位锚点

不是逐像素复刻原文排版。

## 推荐结论

推荐按下面顺序落地：

1. 抽取层骨架
2. `docx`
3. 文本型 `pdf`
4. `read_document_excerpt`
5. `.doc` 外部转换器适配

不推荐的路线：

- 直接扩展现有文本白名单
- 把文件格式解析继续塞进 `rag.rs`
- 把 `.doc` 当成和 `.docx` 同难度问题处理
- 在第一版把 OCR、PDF、DOCX、DOC 全部混成一个统一黑盒

这个需求的正确实现方向不是“支持更多扩展名”，而是把“文档抽取”升级为 RAG 索引的显式前置阶段。
