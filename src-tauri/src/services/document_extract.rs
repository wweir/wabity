use std::{
    collections::HashMap,
    io::{Cursor, Read},
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
};

use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

const SUPPORTED_DOCUMENT_EXTENSIONS: &[&str] =
    &["md", "mdx", "txt", "markdown", "rst", "adoc", "docx", "pdf"];
const MARKDOWN_LIKE_DOCUMENT_EXTENSIONS: &[&str] = &["md", "mdx", "markdown", "docx"];
const MARKDOWN_DOCUMENT_EXTENSIONS: &[&str] = &["md", "mdx", "markdown"];
const PLAIN_TEXT_DOCUMENT_EXTENSIONS: &[&str] = &["txt", "rst", "adoc"];
const PLAIN_TEXT_EXTRACTOR_FINGERPRINT: &str = "plain-text/v1";
const DOCX_EXTRACTOR_FINGERPRINT: &str = "docx/v1";
const PDF_EXTRACTOR_FINGERPRINT: &str = "pdf-text/v1";
const PDF_BLOCK_TARGET_CHARS: usize = 700;
const PDF_BLOCK_MIN_SENTENCE_CHARS: usize = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    PlainText,
    Markdown,
    Pdf,
    Docx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedDocument {
    pub kind: DocumentKind,
    pub extractor_fingerprint: String,
    pub normalized_text: String,
    pub blocks: Vec<ExtractedBlock>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedBlock {
    pub text: String,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub heading_path: Vec<String>,
    pub anchor_label: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
}

pub(crate) fn classify_document_kind(path: &Path) -> Option<DocumentKind> {
    if path_has_extension(path, MARKDOWN_DOCUMENT_EXTENSIONS) {
        return Some(DocumentKind::Markdown);
    }
    if path_has_extension(path, PLAIN_TEXT_DOCUMENT_EXTENSIONS) {
        return Some(DocumentKind::PlainText);
    }
    if path_has_extension(path, &["docx"]) {
        return Some(DocumentKind::Docx);
    }
    if path_has_extension(path, &["pdf"]) {
        return Some(DocumentKind::Pdf);
    }
    None
}

pub(crate) fn is_supported_document_file(path: &Path) -> bool {
    path_has_extension(path, SUPPORTED_DOCUMENT_EXTENSIONS)
}

pub(crate) fn uses_markdown_chunking(path: &Path) -> bool {
    path_has_extension(path, MARKDOWN_LIKE_DOCUMENT_EXTENSIONS)
}

pub(crate) fn extract_document_from_bytes(path: &Path, bytes: &[u8]) -> Result<ExtractedDocument> {
    match classify_document_kind(path) {
        Some(DocumentKind::PlainText) | Some(DocumentKind::Markdown) => {
            extract_plain_text_document(path, bytes)
        }
        Some(DocumentKind::Docx) => extract_docx_document(bytes),
        Some(DocumentKind::Pdf) => extract_pdf_document(path, bytes),
        None => bail!("unsupported document type: {}", path.display()),
    }
    .with_context(|| format!("document extraction failed: {}", path.display()))
}

pub(crate) fn load_readable_document_text(path: &Path) -> Result<String> {
    load_extracted_document(path).map(|document| document.normalized_text)
}

pub(crate) fn load_extracted_document(path: &Path) -> Result<ExtractedDocument> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    extract_document_from_bytes(path, &bytes)
}

fn extract_plain_text_document(path: &Path, bytes: &[u8]) -> Result<ExtractedDocument> {
    if bytes.contains(&0) {
        bail!("file contains NUL bytes and cannot be indexed as text");
    }

    let normalized_text = String::from_utf8(bytes.to_vec())
        .with_context(|| format!("file is not valid UTF-8 text: {}", path.display()))?;
    let kind = if path_has_extension(path, MARKDOWN_DOCUMENT_EXTENSIONS) {
        DocumentKind::Markdown
    } else {
        DocumentKind::PlainText
    };

    Ok(build_extracted_document(
        kind,
        PLAIN_TEXT_EXTRACTOR_FINGERPRINT,
        normalized_text,
        Vec::new(),
        Vec::new(),
    ))
}

fn extract_docx_document(bytes: &[u8]) -> Result<ExtractedDocument> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).context("failed to open DOCX zip archive")?;
    let document_xml = read_docx_entry_to_string(&mut archive, "word/document.xml")
        .context("DOCX 缺少 word/document.xml")?;
    let heading_styles = read_optional_docx_entry_to_string(&mut archive, "word/styles.xml")?
        .map(|xml| parse_docx_heading_styles(&xml))
        .transpose()?
        .unwrap_or_default();

    let normalized_text = render_docx_body_as_markdown(&document_xml, &heading_styles)?;
    Ok(build_extracted_document(
        DocumentKind::Docx,
        DOCX_EXTRACTOR_FINGERPRINT,
        normalized_text,
        Vec::new(),
        Vec::new(),
    ))
}

fn extract_pdf_document(path: &Path, bytes: &[u8]) -> Result<ExtractedDocument> {
    let pages = run_pdf_extract(path, || pdf_extract::extract_text_from_mem_by_pages(bytes))
        .with_context(|| format!("failed to extract text from PDF: {}", path.display()))?;
    if pages.is_empty() {
        bail!("PDF 文档没有可提取的页面: {}", path.display());
    }

    let normalized_pages = pages
        .into_iter()
        .map(|page| normalize_pdf_page_lines(&page))
        .collect::<Vec<_>>();
    let stripped_pages = strip_repeated_pdf_page_noise(&normalized_pages);
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    for (page_index, lines) in stripped_pages.into_iter().enumerate() {
        let page_number = u32::try_from(page_index + 1).context("pdf page number exceeds u32")?;
        if !page_contains_readable_text(&lines) {
            warnings.push(format!("page {page_number} did not produce readable text"));
            continue;
        }

        blocks.extend(split_pdf_page_into_blocks(page_number, &lines));
    }
    if blocks.is_empty() {
        bail!("PDF 文档没有可提取的文本内容: {}", path.display());
    }

    let normalized_text = blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(build_extracted_document(
        DocumentKind::Pdf,
        PDF_EXTRACTOR_FINGERPRINT,
        normalized_text,
        blocks,
        warnings,
    ))
}

fn run_pdf_extract<T>(
    path: &Path,
    operation: impl FnOnce() -> Result<T, pdf_extract::OutputError>,
) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error).context("pdf-extract returned an error"),
        Err(payload) => bail!(
            "pdf-extract panicked while parsing {}: {}",
            path.display(),
            panic_payload_message(&payload)
        ),
    }
}

fn normalize_pdf_page_lines(page_text: &str) -> Vec<String> {
    page_text
        .replace('\u{a0}', " ")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(normalize_pdf_line)
        .collect()
}

fn normalize_pdf_line(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_repeated_pdf_page_noise(pages: &[Vec<String>]) -> Vec<Vec<String>> {
    if pages.len() < 3 {
        return pages.to_vec();
    }

    let required_repeats = pages.len().div_ceil(2);
    let header_counts = collect_pdf_edge_line_counts(pages, true);
    let footer_counts = collect_pdf_edge_line_counts(pages, false);
    pages
        .iter()
        .map(|lines| {
            let mut filtered = lines.clone();
            if let Some(header) = filtered.first() {
                if should_drop_repeated_pdf_edge(header, &header_counts, required_repeats) {
                    filtered.remove(0);
                }
            }
            if let Some(footer) = filtered.last() {
                if should_drop_repeated_pdf_edge(footer, &footer_counts, required_repeats) {
                    filtered.pop();
                }
            }
            filtered
        })
        .collect()
}

fn collect_pdf_edge_line_counts(pages: &[Vec<String>], first: bool) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for line in pages.iter().filter_map(|lines| {
        if first {
            lines.iter().find(|line| !line.is_empty())
        } else {
            lines.iter().rev().find(|line| !line.is_empty())
        }
    }) {
        *counts.entry(line.clone()).or_insert(0) += 1;
    }
    counts
}

fn should_drop_repeated_pdf_edge(
    line: &str,
    counts: &HashMap<String, usize>,
    required_repeats: usize,
) -> bool {
    !line.is_empty() && counts.get(line).copied().unwrap_or(0) >= required_repeats
}

fn page_contains_readable_text(lines: &[String]) -> bool {
    lines.iter().any(|line| !line.trim().is_empty())
}

fn split_pdf_page_into_blocks(page_number: u32, lines: &[String]) -> Vec<ExtractedBlock> {
    let mut blocks = Vec::new();
    let mut current_lines = Vec::new();
    let mut current_chars = 0usize;

    for line in lines {
        if line.is_empty() {
            flush_pdf_block(
                page_number,
                &mut current_lines,
                &mut current_chars,
                &mut blocks,
            );
            continue;
        }

        current_chars = current_chars.saturating_add(line.chars().count());
        current_lines.push(line.clone());
        if should_flush_pdf_block(current_chars, line) {
            flush_pdf_block(
                page_number,
                &mut current_lines,
                &mut current_chars,
                &mut blocks,
            );
        }
    }

    flush_pdf_block(
        page_number,
        &mut current_lines,
        &mut current_chars,
        &mut blocks,
    );
    blocks
}

fn should_flush_pdf_block(current_chars: usize, line: &str) -> bool {
    current_chars >= PDF_BLOCK_TARGET_CHARS
        || (line_ends_sentence(line) && current_chars >= PDF_BLOCK_MIN_SENTENCE_CHARS)
}

fn line_ends_sentence(line: &str) -> bool {
    line.chars().last().is_some_and(|character| {
        matches!(
            character,
            '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '；' | '：'
        )
    })
}

fn flush_pdf_block(
    page_number: u32,
    current_lines: &mut Vec<String>,
    current_chars: &mut usize,
    blocks: &mut Vec<ExtractedBlock>,
) {
    if current_lines.is_empty() {
        *current_chars = 0;
        return;
    }

    let text = current_lines.join("\n").trim().to_string();
    current_lines.clear();
    *current_chars = 0;
    if text.is_empty() {
        return;
    }

    blocks.push(ExtractedBlock {
        text,
        page_start: Some(page_number),
        page_end: Some(page_number),
        heading_path: Vec::new(),
        anchor_label: Some(format!("page {page_number}")),
        line_start: None,
        line_end: None,
    });
}

fn build_extracted_document(
    kind: DocumentKind,
    extractor_fingerprint: &str,
    normalized_text: String,
    blocks: Vec<ExtractedBlock>,
    warnings: Vec<String>,
) -> ExtractedDocument {
    ExtractedDocument {
        kind,
        extractor_fingerprint: extractor_fingerprint.to_string(),
        normalized_text,
        blocks,
        warnings,
    }
}

fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }

    "unknown panic payload".to_string()
}

fn path_has_extension(path: &Path, supported_extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            supported_extensions
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn read_docx_entry_to_string(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    entry_name: &str,
) -> Result<String> {
    let mut file = archive
        .by_name(entry_name)
        .with_context(|| format!("missing DOCX entry: {entry_name}"))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("failed to read DOCX entry as UTF-8 XML: {entry_name}"))?;
    Ok(contents)
}

fn read_optional_docx_entry_to_string(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    entry_name: &str,
) -> Result<Option<String>> {
    match archive.by_name(entry_name) {
        Ok(mut file) => {
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .with_context(|| format!("failed to read DOCX entry as UTF-8 XML: {entry_name}"))?;
            Ok(Some(contents))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to open DOCX entry: {entry_name}"))
        }
    }
}

fn parse_docx_heading_styles(xml: &str) -> Result<HashMap<String, usize>> {
    let document = Document::parse(xml).context("failed to parse DOCX styles.xml")?;
    let mut heading_styles = HashMap::new();

    for style in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "style")
    {
        let style_type = attribute_value(style, "type");
        if style_type.as_deref() != Some("paragraph") {
            continue;
        }

        let Some(style_id) = attribute_value(style, "styleId") else {
            continue;
        };
        let level = extract_heading_level(&style_id).or_else(|| {
            style
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "name")
                .and_then(|node| attribute_value(node, "val"))
                .and_then(|name| extract_heading_level(&name))
        });

        if let Some(level) = level {
            heading_styles.insert(style_id.to_ascii_lowercase(), level);
        }
    }

    Ok(heading_styles)
}

fn render_docx_body_as_markdown(
    document_xml: &str,
    heading_styles: &HashMap<String, usize>,
) -> Result<String> {
    let document = Document::parse(document_xml).context("failed to parse DOCX document.xml")?;
    let body = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "body")
        .context("DOCX document.xml 缺少 body")?;

    let mut blocks = Vec::new();
    collect_docx_blocks(body, heading_styles, &mut blocks);

    let rendered = blocks
        .into_iter()
        .map(|block| block.trim().to_string())
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if rendered.trim().is_empty() {
        bail!("DOCX 文档没有可提取的文本内容");
    }

    Ok(rendered)
}

fn collect_docx_blocks(
    node: Node<'_, '_>,
    heading_styles: &HashMap<String, usize>,
    blocks: &mut Vec<String>,
) {
    for child in node.children().filter(|child| child.is_element()) {
        match child.tag_name().name() {
            "p" => {
                if let Some(paragraph) = render_docx_paragraph(child, heading_styles, false) {
                    blocks.push(paragraph);
                }
            }
            "tbl" => collect_docx_table_rows(child, heading_styles, blocks),
            _ => collect_docx_blocks(child, heading_styles, blocks),
        }
    }
}

fn collect_docx_table_rows(
    table: Node<'_, '_>,
    heading_styles: &HashMap<String, usize>,
    blocks: &mut Vec<String>,
) {
    for row in table
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "tr")
    {
        let cells = row
            .children()
            .filter(|child| child.is_element() && child.tag_name().name() == "tc")
            .filter_map(|cell| render_docx_table_cell(cell, heading_styles))
            .collect::<Vec<_>>();

        if !cells.is_empty() {
            blocks.push(cells.join(" | "));
        }
    }
}

fn render_docx_table_cell(
    cell: Node<'_, '_>,
    heading_styles: &HashMap<String, usize>,
) -> Option<String> {
    let paragraphs = cell
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "p")
        .filter_map(|paragraph| render_docx_paragraph(paragraph, heading_styles, true))
        .collect::<Vec<_>>();
    let text = paragraphs.join(" / ");
    (!text.trim().is_empty()).then_some(text)
}

fn render_docx_paragraph(
    paragraph: Node<'_, '_>,
    heading_styles: &HashMap<String, usize>,
    inside_table: bool,
) -> Option<String> {
    let raw_text = collect_docx_paragraph_text(paragraph);
    let text = normalize_docx_text(&raw_text);
    if text.is_empty() {
        return None;
    }

    if inside_table {
        return Some(text);
    }

    let heading_level = paragraph_heading_level(paragraph, heading_styles);
    if let Some(level) = heading_level {
        let hashes = "#".repeat(level.clamp(1, 6));
        return Some(format!("{hashes} {text}"));
    }

    if paragraph_is_list_item(paragraph) {
        return Some(format!("- {text}"));
    }

    Some(text)
}

fn paragraph_heading_level(
    paragraph: Node<'_, '_>,
    heading_styles: &HashMap<String, usize>,
) -> Option<usize> {
    paragraph
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "pPr")
        .and_then(|properties| {
            properties
                .children()
                .find(|child| child.is_element() && child.tag_name().name() == "pStyle")
        })
        .and_then(|style| attribute_value(style, "val"))
        .and_then(|style_id| {
            heading_styles
                .get(&style_id.to_ascii_lowercase())
                .copied()
                .or_else(|| extract_heading_level(&style_id))
        })
}

fn paragraph_is_list_item(paragraph: Node<'_, '_>) -> bool {
    paragraph
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "pPr")
        .is_some_and(|properties| {
            properties
                .descendants()
                .any(|node| node.is_element() && node.tag_name().name() == "numPr")
        })
}

fn collect_docx_paragraph_text(paragraph: Node<'_, '_>) -> String {
    let mut text = String::new();

    for node in paragraph.descendants().filter(|node| node.is_element()) {
        match node.tag_name().name() {
            "t" => {
                if let Some(value) = node.text() {
                    text.push_str(value);
                }
            }
            "tab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            _ => {}
        }
    }

    text
}

fn normalize_docx_text(raw: &str) -> String {
    let replaced = raw.replace('\u{a0}', " ");
    let normalized_lines = replaced
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    normalized_lines.join("\n")
}

fn extract_heading_level(value: &str) -> Option<usize> {
    let normalized = value
        .chars()
        .filter(|character| {
            !character.is_ascii_whitespace() && *character != '-' && *character != '_'
        })
        .collect::<String>()
        .to_ascii_lowercase();
    let suffix = normalized.strip_prefix("heading")?;
    suffix.parse::<usize>().ok().filter(|level| *level > 0)
}

fn attribute_value(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.attributes()
        .find(|attribute| attribute.name() == name)
        .map(|attribute| attribute.value().to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn build_test_docx(document_xml: &str, styles_xml: Option<&str>) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        writer
            .start_file("word/document.xml", options)
            .expect("start document.xml");
        writer
            .write_all(document_xml.as_bytes())
            .expect("write document.xml");

        if let Some(styles_xml) = styles_xml {
            writer
                .start_file("word/styles.xml", options)
                .expect("start styles.xml");
            writer
                .write_all(styles_xml.as_bytes())
                .expect("write styles.xml");
        }

        writer.finish().expect("finish docx writer").into_inner()
    }

    fn simple_test_pdf_bytes() -> Vec<u8> {
        let content_stream = "\
BT
/F1 12 Tf
72 100 Td
(Hello PDF extraction.) Tj
0 -18 Td
(Second line on page one.) Tj
ET";
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n".to_string(),
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                content_stream.len(),
                content_stream
            ),
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n"
                .to_string(),
        ];

        let mut pdf = String::from("%PDF-1.4\n");
        let mut object_offsets = Vec::with_capacity(objects.len() + 1);
        object_offsets.push(0usize);
        for object in objects {
            object_offsets.push(pdf.len());
            pdf.push_str(&object);
        }

        let startxref = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n", object_offsets.len()));
        pdf.push_str("0000000000 65535 f \n");
        for offset in object_offsets.iter().skip(1) {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{}\n%%EOF",
            object_offsets.len(),
            startxref
        ));
        pdf.into_bytes()
    }

    #[test]
    fn supported_document_extensions_include_docx_and_pdf() {
        assert!(is_supported_document_file(Path::new("/tmp/notes.docx")));
        assert!(is_supported_document_file(Path::new("/tmp/notes.pdf")));
        assert!(uses_markdown_chunking(Path::new("/tmp/notes.docx")));
        assert!(!uses_markdown_chunking(Path::new("/tmp/notes.pdf")));
    }

    #[test]
    fn docx_extraction_renders_headings_lists_and_tables() {
        let document_xml = r#"
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p>
                  <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
                  <w:r><w:t>Architecture</w:t></w:r>
                </w:p>
                <w:p>
                  <w:r><w:t>Alpha paragraph.</w:t></w:r>
                </w:p>
                <w:p>
                  <w:pPr><w:numPr/></w:pPr>
                  <w:r><w:t>First item</w:t></w:r>
                </w:p>
                <w:tbl>
                  <w:tr>
                    <w:tc><w:p><w:r><w:t>Key</w:t></w:r></w:p></w:tc>
                    <w:tc><w:p><w:r><w:t>Value</w:t></w:r></w:p></w:tc>
                  </w:tr>
                </w:tbl>
              </w:body>
            </w:document>
        "#;

        let text = extract_docx_document(&build_test_docx(document_xml, None))
            .expect("extract docx text")
            .normalized_text;

        assert!(text.contains("# Architecture"));
        assert!(text.contains("Alpha paragraph."));
        assert!(text.contains("- First item"));
        assert!(text.contains("Key | Value"));
    }

    #[test]
    fn docx_styles_xml_can_map_custom_heading_style_ids() {
        let document_xml = r#"
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p>
                  <w:pPr><w:pStyle w:val="CustomTitle"/></w:pPr>
                  <w:r><w:t>Overview</w:t></w:r>
                </w:p>
              </w:body>
            </w:document>
        "#;
        let styles_xml = r#"
            <w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:style w:type="paragraph" w:styleId="CustomTitle">
                <w:name w:val="Heading 2"/>
              </w:style>
            </w:styles>
        "#;

        let text = extract_docx_document(&build_test_docx(document_xml, Some(styles_xml)))
            .expect("extract styled docx")
            .normalized_text;

        assert!(text.contains("## Overview"));
    }

    #[test]
    fn pdf_extraction_returns_page_anchored_blocks() {
        let extracted = extract_pdf_document(Path::new("/tmp/test.pdf"), &simple_test_pdf_bytes())
            .expect("extract simple pdf text");

        assert_eq!(extracted.kind, DocumentKind::Pdf);
        assert_eq!(extracted.blocks.len(), 1);
        assert_eq!(extracted.blocks[0].page_start, Some(1));
        assert_eq!(extracted.blocks[0].page_end, Some(1));
        assert!(extracted.normalized_text.contains("Hello PDF extraction."));
        assert!(extracted
            .normalized_text
            .contains("Second line on page one."));
    }

    #[test]
    fn pdf_extract_panic_is_converted_into_error() {
        let path = Path::new("/tmp/broken.pdf");
        let error = run_pdf_extract(
            path,
            || -> std::result::Result<Vec<String>, pdf_extract::OutputError> {
                panic!("boom");
            },
        )
        .expect_err("panic should be converted into error");

        let message = error.to_string();
        assert!(message.contains("pdf-extract panicked while parsing /tmp/broken.pdf: boom"));
    }
}
