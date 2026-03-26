use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::Path,
};

use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node};
use zip::ZipArchive;

const SUPPORTED_DOCUMENT_EXTENSIONS: &[&str] =
    &["md", "mdx", "txt", "markdown", "rst", "adoc", "docx"];
const MARKDOWN_LIKE_DOCUMENT_EXTENSIONS: &[&str] = &["md", "mdx", "markdown", "docx"];

pub(crate) fn is_supported_document_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SUPPORTED_DOCUMENT_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

pub(crate) fn uses_markdown_chunking(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            MARKDOWN_LIKE_DOCUMENT_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

pub(crate) fn extract_document_text_from_bytes(path: &Path, bytes: &[u8]) -> Result<String> {
    if is_docx_file(path) {
        return extract_docx_text_from_bytes(bytes);
    }

    if bytes.contains(&0) {
        bail!("file contains NUL bytes and cannot be indexed as text");
    }

    String::from_utf8(bytes.to_vec())
        .with_context(|| format!("file is not valid UTF-8 text: {}", path.display()))
}

pub(crate) fn load_readable_document_text(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    extract_document_text_from_bytes(path, &bytes)
}

fn is_docx_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
}

fn extract_docx_text_from_bytes(bytes: &[u8]) -> Result<String> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).context("failed to open DOCX zip archive")?;
    let document_xml = read_docx_entry_to_string(&mut archive, "word/document.xml")
        .context("DOCX 缺少 word/document.xml")?;
    let heading_styles = read_optional_docx_entry_to_string(&mut archive, "word/styles.xml")?
        .map(|xml| parse_docx_heading_styles(&xml))
        .transpose()?
        .unwrap_or_default();

    render_docx_body_as_markdown(&document_xml, &heading_styles)
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

    #[test]
    fn supported_document_extensions_include_docx() {
        assert!(is_supported_document_file(Path::new("/tmp/notes.docx")));
        assert!(uses_markdown_chunking(Path::new("/tmp/notes.docx")));
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

        let text = extract_docx_text_from_bytes(&build_test_docx(document_xml, None))
            .expect("extract docx text");

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

        let text = extract_docx_text_from_bytes(&build_test_docx(document_xml, Some(styles_xml)))
            .expect("extract styled docx");

        assert!(text.contains("## Overview"));
    }
}
