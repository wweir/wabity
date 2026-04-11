use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::Path,
};

use anyhow::{bail, Context, Result};
use lopdf::Document as PdfDocument;
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
const PDF_EXTRACTOR_FINGERPRINT: &str = "pdf-text/v3";
const PDF_BLOCK_TARGET_CHARS: usize = 700;
const PDF_BLOCK_MIN_SENTENCE_CHARS: usize = 280;
const PDF_MIN_INDEXABLE_NON_WHITESPACE_CHARS: usize = 24;
const PDF_MIN_WORDLIKE_CHAR_RATIO_PERCENT: usize = 45;
const PDF_MAX_SUSPICIOUS_CHAR_RATIO_PERCENT: usize = 10;
const PDF_SIGNIFICANT_SCRIPT_CHAR_COUNT: usize = 6;
const PDF_SIGNIFICANT_SCRIPT_RATIO_PERCENT: usize = 12;
const MAX_REPORTED_PDF_WARNING_PAGES: usize = 3;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExtractedPdfPage {
    page_number: u32,
    lines: Vec<String>,
    extraction_warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct PdfWarningRollup {
    extraction_warning_count: usize,
    extraction_warning_pages: Vec<u32>,
    unreadable_pages: Vec<u32>,
    corrupted_pages: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PdfTextQualityStats {
    non_whitespace_chars: usize,
    wordlike_chars: usize,
    suspicious_chars: usize,
    latin_chars: usize,
    cjk_chars: usize,
    cyrillic_chars: usize,
    arabic_chars: usize,
    hebrew_chars: usize,
    greek_chars: usize,
    other_letter_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfScriptBucket {
    Latin,
    Cjk,
    Cyrillic,
    Arabic,
    Hebrew,
    Greek,
    OtherLetter,
}

impl ExtractedPdfPage {
    fn from_chunks(
        page_number: u32,
        text_chunks: Vec<String>,
        extraction_warnings: Vec<String>,
    ) -> Self {
        let page_text = text_chunks.join("\n");
        Self {
            page_number,
            lines: normalize_pdf_page_lines(&page_text),
            extraction_warnings,
        }
    }
}

impl PdfWarningRollup {
    fn record_extraction_warnings(&mut self, page_number: u32, warning_count: usize) {
        if warning_count == 0 {
            return;
        }
        self.extraction_warning_count += warning_count;
        self.extraction_warning_pages.push(page_number);
    }

    fn record_unreadable_page(&mut self, page_number: u32) {
        self.unreadable_pages.push(page_number);
    }

    fn record_corrupted_page(&mut self, page_number: u32) {
        self.corrupted_pages.push(page_number);
    }

    fn to_warning_messages(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if self.extraction_warning_count > 0 {
            parts.push(format!(
                "{} text extraction warning(s) across {} page(s){}",
                self.extraction_warning_count,
                self.extraction_warning_pages.len(),
                format_pdf_warning_page_suffix(&self.extraction_warning_pages),
            ));
        }
        if !self.unreadable_pages.is_empty() {
            parts.push(format!(
                "{} page(s) did not produce readable text{}",
                self.unreadable_pages.len(),
                format_pdf_warning_page_suffix(&self.unreadable_pages),
            ));
        }
        if !self.corrupted_pages.is_empty() {
            parts.push(format!(
                "{} page(s) looked corrupted and were skipped{}",
                self.corrupted_pages.len(),
                format_pdf_warning_page_suffix(&self.corrupted_pages),
            ));
        }

        if parts.is_empty() {
            Vec::new()
        } else {
            vec![format!("PDF extraction summary: {}", parts.join("; "))]
        }
    }
}

fn format_pdf_warning_page_suffix(page_numbers: &[u32]) -> String {
    let sample = page_numbers
        .iter()
        .take(MAX_REPORTED_PDF_WARNING_PAGES)
        .map(u32::to_string)
        .collect::<Vec<_>>();
    if sample.is_empty() {
        return String::new();
    }

    let suffix = if page_numbers.len() > sample.len() {
        ", ..."
    } else {
        ""
    };
    format!(" (pages {}{})", sample.join(", "), suffix)
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

pub(crate) fn extractor_fingerprint_for_path(path: &Path) -> Option<&'static str> {
    match classify_document_kind(path) {
        Some(DocumentKind::PlainText) | Some(DocumentKind::Markdown) => {
            Some(PLAIN_TEXT_EXTRACTOR_FINGERPRINT)
        }
        Some(DocumentKind::Docx) => Some(DOCX_EXTRACTOR_FINGERPRINT),
        Some(DocumentKind::Pdf) => Some(PDF_EXTRACTOR_FINGERPRINT),
        None => None,
    }
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
    let pages = extract_pdf_pages(path, bytes)
        .with_context(|| format!("failed to extract text from PDF: {}", path.display()))?;
    let normalized_pages = pages
        .iter()
        .map(|page| page.lines.clone())
        .collect::<Vec<_>>();
    let stripped_pages = strip_repeated_pdf_page_noise(&normalized_pages);
    let mut blocks = Vec::new();
    let mut warning_rollup = PdfWarningRollup::default();
    for (page, lines) in pages.iter().zip(stripped_pages) {
        let page_number = page.page_number;
        warning_rollup.record_extraction_warnings(page_number, page.extraction_warnings.len());
        if !page_contains_any_text(&lines) {
            warning_rollup.record_unreadable_page(page_number);
            continue;
        }
        if !page_contains_readable_text(&lines) {
            warning_rollup.record_corrupted_page(page_number);
            continue;
        }

        blocks.extend(split_pdf_page_into_blocks(page_number, &lines));
    }
    let warnings = warning_rollup.to_warning_messages();
    if blocks.is_empty() {
        let detail = warnings
            .last()
            .map(|warning| format!(" ({warning})"))
            .unwrap_or_default();
        bail!("PDF 文档没有可提取的文本内容: {}{}", path.display(), detail);
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

fn extract_pdf_pages(path: &Path, bytes: &[u8]) -> Result<Vec<ExtractedPdfPage>> {
    let mut document = PdfDocument::load_mem(bytes).context("failed to parse PDF bytes")?;
    if document.is_encrypted() {
        document
            .decrypt("")
            .context("encrypted PDF requires a password")?;
    }

    let page_numbers = document.get_pages().into_keys().collect::<Vec<_>>();
    if page_numbers.is_empty() {
        bail!("PDF 文档没有可提取的页面: {}", path.display());
    }

    Ok(page_numbers
        .into_iter()
        .map(|page_number| extract_pdf_page(&document, page_number))
        .collect())
}

fn extract_pdf_page(document: &PdfDocument, page_number: u32) -> ExtractedPdfPage {
    let mut text_chunks = Vec::new();
    let mut extraction_warnings = Vec::new();
    for chunk in document.extract_text_chunks(&[page_number]) {
        match chunk {
            Ok(text) => {
                if !text.trim().is_empty() {
                    text_chunks.push(text);
                }
            }
            Err(error) => extraction_warnings.push(format!(
                "page {page_number} text extraction warning: {error}"
            )),
        }
    }
    ExtractedPdfPage::from_chunks(page_number, text_chunks, extraction_warnings)
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

fn page_contains_any_text(lines: &[String]) -> bool {
    lines.iter().any(|line| !line.trim().is_empty())
}

fn page_contains_readable_text(lines: &[String]) -> bool {
    let stats = collect_pdf_text_quality_stats(lines);
    if stats.non_whitespace_chars == 0 {
        return false;
    }
    if stats.non_whitespace_chars < PDF_MIN_INDEXABLE_NON_WHITESPACE_CHARS {
        return stats.wordlike_chars > 0 && stats.suspicious_chars == 0;
    }

    stats.wordlike_chars * 100 >= stats.non_whitespace_chars * PDF_MIN_WORDLIKE_CHAR_RATIO_PERCENT
        && stats.suspicious_chars * 100
            <= stats.non_whitespace_chars * PDF_MAX_SUSPICIOUS_CHAR_RATIO_PERCENT
        && !has_suspicious_pdf_script_mix(&stats)
}

fn collect_pdf_text_quality_stats(lines: &[String]) -> PdfTextQualityStats {
    let mut stats = PdfTextQualityStats::default();
    for character in lines.iter().flat_map(|line| line.chars()) {
        if character.is_whitespace() {
            continue;
        }
        stats.non_whitespace_chars += 1;
        if is_wordlike_pdf_char(character) {
            stats.wordlike_chars += 1;
        }
        if is_suspicious_pdf_char(character) {
            stats.suspicious_chars += 1;
        }
        match classify_pdf_script_bucket(character) {
            Some(PdfScriptBucket::Latin) => stats.latin_chars += 1,
            Some(PdfScriptBucket::Cjk) => stats.cjk_chars += 1,
            Some(PdfScriptBucket::Cyrillic) => stats.cyrillic_chars += 1,
            Some(PdfScriptBucket::Arabic) => stats.arabic_chars += 1,
            Some(PdfScriptBucket::Hebrew) => stats.hebrew_chars += 1,
            Some(PdfScriptBucket::Greek) => stats.greek_chars += 1,
            Some(PdfScriptBucket::OtherLetter) => stats.other_letter_chars += 1,
            None => {}
        }
    }
    stats
}

fn has_suspicious_pdf_script_mix(stats: &PdfTextQualityStats) -> bool {
    let script_chars = stats.latin_chars
        + stats.cjk_chars
        + stats.cyrillic_chars
        + stats.arabic_chars
        + stats.hebrew_chars
        + stats.greek_chars
        + stats.other_letter_chars;
    if script_chars == 0 {
        return false;
    }

    let cjk_is_significant = is_significant_pdf_script_bucket(stats.cjk_chars, script_chars);
    let other_letter_is_significant =
        is_significant_pdf_script_bucket(stats.other_letter_chars, script_chars);
    let significant_named_unexpected_bucket_count = [
        stats.cyrillic_chars,
        stats.arabic_chars,
        stats.hebrew_chars,
        stats.greek_chars,
    ]
    .into_iter()
    .filter(|count| is_significant_pdf_script_bucket(*count, script_chars))
    .count();

    cjk_is_significant
        && (other_letter_is_significant || significant_named_unexpected_bucket_count >= 2)
}

fn is_significant_pdf_script_bucket(bucket_chars: usize, total_script_chars: usize) -> bool {
    bucket_chars >= PDF_SIGNIFICANT_SCRIPT_CHAR_COUNT
        && bucket_chars * 100 >= total_script_chars * PDF_SIGNIFICANT_SCRIPT_RATIO_PERCENT
}

fn classify_pdf_script_bucket(character: char) -> Option<PdfScriptBucket> {
    let code_point = character as u32;
    if is_cjk_script_char(character) {
        return Some(PdfScriptBucket::Cjk);
    }
    if matches!(
        code_point,
        0x0041..=0x024F | 0x1E00..=0x1EFF | 0x2C60..=0x2C7F | 0xA720..=0xA7FF | 0xAB30..=0xAB6F
    ) {
        return Some(PdfScriptBucket::Latin);
    }
    if matches!(code_point, 0x0370..=0x03FF | 0x1F00..=0x1FFF) {
        return Some(PdfScriptBucket::Greek);
    }
    if matches!(
        code_point,
        0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F
    ) {
        return Some(PdfScriptBucket::Cyrillic);
    }
    if matches!(
        code_point,
        0x0590..=0x05FF | 0xFB1D..=0xFB4F
    ) {
        return Some(PdfScriptBucket::Hebrew);
    }
    if matches!(
        code_point,
        0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x0870..=0x089F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
    ) {
        return Some(PdfScriptBucket::Arabic);
    }
    character
        .is_alphabetic()
        .then_some(PdfScriptBucket::OtherLetter)
}

fn is_wordlike_pdf_char(character: char) -> bool {
    character.is_alphanumeric() || is_cjk_unified_ideograph(character)
}

fn is_suspicious_pdf_char(character: char) -> bool {
    character == '\u{fffd}'
        || character.is_control()
        || matches!(
            character as u32,
            0x200B..=0x200F
                | 0x202A..=0x202E
                | 0x2060..=0x206F
                | 0xFFF0..=0xFFFF
                | 0xE000..=0xF8FF
        )
}

fn is_cjk_unified_ideograph(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
    )
}

fn is_cjk_script_char(character: char) -> bool {
    is_cjk_unified_ideograph(character)
        || matches!(
            character as u32,
            0x3040..=0x30FF
                | 0x31A0..=0x31BF
                | 0x31F0..=0x31FF
                | 0x3400..=0x4DBF
                | 0xAC00..=0xD7AF
                | 0x1100..=0x11FF
                | 0x3130..=0x318F
                | 0xA960..=0xA97F
                | 0xD7B0..=0xD7FF
                | 0xFF66..=0xFF9D
        )
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
        build_single_page_pdf(
            "\
BT
/F1 12 Tf
72 100 Td
(Hello PDF extraction.) Tj
0 -18 Td
(Second line on page one.) Tj
ET",
        )
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

    fn invalid_content_stream_pdf_bytes() -> Vec<u8> {
        build_single_page_pdf(
            "\
BT
Tf
ET",
        )
    }

    fn partially_invalid_content_stream_pdf_bytes() -> Vec<u8> {
        let content_stream = "\
BT
/F1 12 Tf
72 100 Td
(Recovered text before invalid font.) Tj
/FB 12 Tf
0 -18 Td
(This fragment should be skipped.) Tj
ET";
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /FB 6 0 R >> >> >>\nendobj\n".to_string(),
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                content_stream.len(),
                content_stream
            ),
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n"
                .to_string(),
            "6 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /HeiseiKakuGo-W5 /Encoding /Identity-H >>\nendobj\n".to_string(),
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

    fn build_single_page_pdf(content_stream: &str) -> Vec<u8> {
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
    fn pdf_page_extraction_failures_become_warnings() {
        let extracted = extract_pdf_document(
            Path::new("/tmp/invalid-content.pdf"),
            &invalid_content_stream_pdf_bytes(),
        )
        .expect_err("invalid content stream should not produce readable text");

        let message = extracted.to_string();
        assert!(message.contains("PDF 文档没有可提取的文本内容"));
        assert!(message.contains("PDF extraction summary"));
        assert!(message.contains("1 text extraction warning(s) across 1 page(s)"));
        assert!(message.contains("1 page(s) did not produce readable text"));
    }

    #[test]
    fn pdf_page_extraction_surfaces_page_warnings() {
        let pages = extract_pdf_pages(
            Path::new("/tmp/invalid-content.pdf"),
            &invalid_content_stream_pdf_bytes(),
        )
        .expect("parse invalid-content pdf container");

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_number, 1);
        assert!(pages[0].lines.is_empty());
        assert_eq!(pages[0].extraction_warnings.len(), 1);
        assert!(pages[0].extraction_warnings[0].contains("page 1 text extraction warning"));
    }

    #[test]
    fn pdf_page_extraction_keeps_readable_chunks_when_one_chunk_fails() {
        let extracted = extract_pdf_document(
            Path::new("/tmp/partially-invalid.pdf"),
            &partially_invalid_content_stream_pdf_bytes(),
        )
        .expect("partially invalid content stream should still yield readable text");

        assert!(extracted
            .normalized_text
            .contains("Recovered text before invalid font."));
        assert_eq!(extracted.blocks.len(), 1);
        assert_eq!(extracted.warnings.len(), 1);
        assert!(extracted.warnings[0].contains("PDF extraction summary"));
        assert!(extracted.warnings[0].contains("1 text extraction warning(s) across 1 page(s)"));
    }

    #[test]
    fn pdf_warning_rollup_keeps_single_summary_for_many_pages() {
        let mut rollup = PdfWarningRollup::default();
        rollup.record_extraction_warnings(1, 3);
        rollup.record_extraction_warnings(4, 2);
        rollup.record_unreadable_page(7);
        rollup.record_corrupted_page(8);
        rollup.record_corrupted_page(9);
        rollup.record_corrupted_page(10);
        rollup.record_corrupted_page(11);

        let warnings = rollup.to_warning_messages();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("5 text extraction warning(s) across 2 page(s) (pages 1, 4)"));
        assert!(warnings[0].contains("1 page(s) did not produce readable text (pages 7)"));
        assert!(warnings[0]
            .contains("4 page(s) looked corrupted and were skipped (pages 8, 9, 10, ...)"));
    }

    #[test]
    fn pdf_quality_gate_rejects_suspicious_control_heavy_text() {
        assert!(!page_contains_readable_text(&[
            "\u{0001}\u{0002}\u{0003}\u{0004}broken".to_string(),
            "\u{0005}\u{0006}\u{0007}\u{0008}text".to_string(),
        ]));
        assert!(page_contains_readable_text(&[
            "Readable ASCII text for indexing.".to_string(),
            "Second readable line.".to_string(),
        ]));
    }

    #[test]
    fn pdf_quality_gate_rejects_script_mojibake_mix() {
        assert!(!page_contains_readable_text(&[
            "ᵜҖሩᰙᵏ Linux ᫽֌㌫㔏 研究 内核 设计".to_string(),
            "䘉ᱟ ᇂޞ ተҘሯ㍒ 并非 正常 中文 内容".to_string(),
            "ᵜҫӵ֌ ㌖䠽 机制 分析 与 实现 过程".to_string(),
        ]));
    }

    #[test]
    fn pdf_quality_gate_keeps_single_script_documents_indexable() {
        assert!(page_contains_readable_text(&[
            "Нормальный русский текст для PDF индексации.".to_string(),
            "Вторая строка содержит осмысленное описание.".to_string(),
        ]));
        assert!(page_contains_readable_text(&[
            "هذا نص عربي قابل للفهرسة داخل ملف PDF.".to_string(),
            "السطر الثاني يحتوي على محتوى واضح ومقروء.".to_string(),
        ]));
    }
}
