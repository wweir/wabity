use std::path::Path;

use anyhow::{Context, Result};
use text_splitter::{Characters, ChunkCharIndex, ChunkConfig, MarkdownSplitter, TextSplitter};

use crate::services::document_extract::{
    classify_document_kind, load_extracted_document, uses_markdown_chunking, DocumentKind,
    ExtractedBlock, ExtractedDocument,
};

use super::model::{
    DocumentExcerpt, PreparedRagChunk, CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS,
    MARKDOWN_CHUNK_HARD_MAX_CHARS, MARKDOWN_CHUNK_OVERLAP_CHARS, MARKDOWN_CHUNK_TARGET_CHARS,
};

#[derive(Debug)]
pub(super) struct TextLine {
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) content: String,
}

#[derive(Debug)]
pub(super) struct TextLayout {
    pub(super) lines: Vec<TextLine>,
    pub(super) paragraph_start_lines: Vec<usize>,
    pub(super) heading_path_by_line: Vec<Vec<String>>,
}

#[derive(Debug)]
pub(super) struct ChunkMetadata {
    pub(super) paragraph_start_line_index: usize,
    pub(super) heading_path: Vec<String>,
}

#[derive(Debug, Clone)]
struct ChunkByteRange {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Clone)]
struct SemanticBlock {
    start_byte: usize,
    end_byte: usize,
    char_count: usize,
    heading_path: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkdownFence {
    marker: char,
    length: usize,
}

pub(super) fn build_chunk_config(
    capacity: usize,
    overlap: usize,
) -> Result<ChunkConfig<Characters>> {
    ChunkConfig::new(capacity)
        .with_overlap(overlap)
        .context("invalid text splitter overlap configuration")
}

pub(super) fn split_extracted_document_for_path(
    path: &Path,
    document: &ExtractedDocument,
    capacity: usize,
    overlap: usize,
) -> Result<Vec<PreparedRagChunk>> {
    if document.kind == DocumentKind::Pdf {
        return pack_pdf_blocks(&document.blocks, capacity);
    }

    let text = document.normalized_text.as_str();
    let layout = build_text_layout(text, uses_markdown_chunking(path));
    let mut chunks = if uses_markdown_chunking(path) {
        split_markdown_text(
            text,
            &layout,
            MARKDOWN_CHUNK_TARGET_CHARS,
            MARKDOWN_CHUNK_HARD_MAX_CHARS,
            MARKDOWN_CHUNK_OVERLAP_CHARS,
        )?
    } else {
        build_chunks_from_offsets(
            TextSplitter::new(build_chunk_config(capacity, overlap)?).chunk_char_indices(text),
            &layout,
        )?
    };
    for chunk in &mut chunks {
        chunk.document_kind = classify_document_kind(path).unwrap_or(document.kind);
    }
    Ok(chunks)
}

pub(crate) fn load_document_excerpt_for_chunk(
    path: &Path,
    chunk_index: i32,
) -> Result<DocumentExcerpt> {
    let extracted = load_extracted_document(path)?;
    let chunk =
        split_extracted_document_for_path(path, &extracted, CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS)?
            .into_iter()
            .find(|chunk| chunk.chunk_index == chunk_index)
            .with_context(|| {
                format!(
                    "chunk index {chunk_index} does not exist for {}",
                    path.display()
                )
            })?;

    Ok(DocumentExcerpt {
        document_kind: chunk.document_kind,
        chunk_index: chunk.chunk_index,
        text: chunk.text,
        line_start: chunk.line_start,
        line_end: chunk.line_end,
        paragraph_line_start: chunk.paragraph_line_start,
        page_start: chunk.page_start,
        page_end: chunk.page_end,
        heading_path: chunk.heading_path,
        anchor_label: chunk.anchor_label,
    })
}

fn build_chunks_from_offsets<'text>(
    chunks: impl Iterator<Item = ChunkCharIndex<'text>>,
    layout: &TextLayout,
) -> Result<Vec<PreparedRagChunk>> {
    chunks
        .enumerate()
        .filter(|(_, chunk)| !chunk.chunk.is_empty())
        .map(|(chunk_index, chunk)| {
            build_chunk_from_byte_range(
                layout,
                chunk_index,
                chunk.byte_offset,
                chunk.byte_offset.saturating_add(chunk.chunk.len()),
                chunk.chunk,
            )
        })
        .collect()
}

fn build_chunk_from_byte_range(
    layout: &TextLayout,
    chunk_index: usize,
    start_byte: usize,
    end_byte: usize,
    text: &str,
) -> Result<PreparedRagChunk> {
    let start_line_index = line_index_for_offset(layout, start_byte);
    let end_offset = end_byte.saturating_sub(1);
    let end_line_index = line_index_for_offset(layout, end_offset);
    let chunk_metadata = resolve_chunk_metadata(layout, start_line_index, end_line_index)?;

    Ok(PreparedRagChunk {
        document_kind: DocumentKind::PlainText,
        chunk_index: i32::try_from(chunk_index).context("chunk index exceeds i32 range")?,
        line_start: Some(
            i32::try_from(start_line_index + 1).context("chunk start line exceeds i32 range")?,
        ),
        line_end: Some(
            i32::try_from(end_line_index + 1).context("chunk end line exceeds i32 range")?,
        ),
        paragraph_line_start: Some(
            i32::try_from(chunk_metadata.paragraph_start_line_index + 1)
                .context("paragraph start line exceeds i32 range")?,
        ),
        page_start: None,
        page_end: None,
        chunk_reuse_key: chunk_reuse_key(text, &chunk_metadata.heading_path),
        heading_path: chunk_metadata.heading_path,
        anchor_label: None,
        text: text.to_string(),
    })
}

fn pack_pdf_blocks(blocks: &[ExtractedBlock], capacity: usize) -> Result<Vec<PreparedRagChunk>> {
    if blocks.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks = Vec::new();
    let mut current_blocks = Vec::<&ExtractedBlock>::new();
    let mut current_chars = 0usize;
    let mut current_page = None;

    for block in blocks {
        let block_chars = block.text.chars().count();
        let block_page = block.page_start.or(block.page_end);
        let would_cross_page = current_page.is_some() && block_page != current_page;
        let would_exceed_capacity = !current_blocks.is_empty()
            && current_chars.saturating_add(block_chars).saturating_add(2) > capacity;
        if would_cross_page || would_exceed_capacity {
            chunks.push(build_pdf_chunk(chunks.len(), &current_blocks)?);
            current_blocks.clear();
            current_chars = 0;
        }

        current_chars = current_chars.saturating_add(block_chars).saturating_add(2);
        current_page = block_page;
        current_blocks.push(block);
    }

    if !current_blocks.is_empty() {
        chunks.push(build_pdf_chunk(chunks.len(), &current_blocks)?);
    }

    Ok(chunks)
}

fn build_pdf_chunk(chunk_index: usize, blocks: &[&ExtractedBlock]) -> Result<PreparedRagChunk> {
    let first = blocks.first().context("missing first PDF block")?;
    let last = blocks.last().context("missing last PDF block")?;
    let text = blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let heading_path = common_heading_path(blocks.iter().map(|block| block.heading_path.clone()));
    let anchor_label = if first.anchor_label == last.anchor_label {
        first.anchor_label.clone()
    } else {
        None
    };

    Ok(PreparedRagChunk {
        document_kind: DocumentKind::Pdf,
        chunk_index: i32::try_from(chunk_index).context("chunk index exceeds i32 range")?,
        line_start: None,
        line_end: None,
        paragraph_line_start: None,
        page_start: first.page_start.and_then(|page| i32::try_from(page).ok()),
        page_end: last.page_end.and_then(|page| i32::try_from(page).ok()),
        heading_path: heading_path.clone(),
        anchor_label,
        chunk_reuse_key: chunk_reuse_key(&text, &heading_path),
        text,
    })
}

fn common_heading_path(mut paths: impl Iterator<Item = Vec<String>>) -> Vec<String> {
    let Some(mut prefix) = paths.next() else {
        return Vec::new();
    };
    for path in paths {
        let shared = prefix
            .iter()
            .zip(path.iter())
            .take_while(|(left, right)| left == right)
            .count();
        prefix.truncate(shared);
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

fn split_markdown_text(
    text: &str,
    layout: &TextLayout,
    target_chars: usize,
    hard_max_chars: usize,
    overlap_chars: usize,
) -> Result<Vec<PreparedRagChunk>> {
    let semantic_blocks = collect_markdown_semantic_blocks(layout)?;
    if semantic_blocks.is_empty() {
        return Ok(Vec::new());
    }

    let ranges = pack_markdown_blocks(
        text,
        &semantic_blocks,
        target_chars,
        hard_max_chars,
        overlap_chars,
    )?;

    ranges
        .into_iter()
        .enumerate()
        .map(|(chunk_index, range)| {
            let chunk_text = &text[range.start_byte..range.end_byte];
            build_chunk_from_byte_range(
                layout,
                chunk_index,
                range.start_byte,
                range.end_byte,
                chunk_text,
            )
        })
        .collect()
}

fn collect_markdown_semantic_blocks(layout: &TextLayout) -> Result<Vec<SemanticBlock>> {
    let mut blocks = Vec::new();
    let mut line_index = 0usize;

    while line_index < layout.lines.len() {
        if layout.lines[line_index].content.trim().is_empty() {
            line_index += 1;
            continue;
        }

        let trimmed = layout.lines[line_index].content.trim();
        let end_line_index = if let Some(fence) = parse_markdown_fence(trimmed) {
            find_markdown_fence_end(layout, line_index, fence)
        } else if parse_atx_heading(trimmed).is_some() {
            line_index
        } else if parse_setext_heading(&layout.lines, line_index).is_some() {
            line_index
                .saturating_add(1)
                .min(layout.lines.len().saturating_sub(1))
        } else if parse_markdown_list_item(trimmed).is_some() {
            find_list_item_end(layout, line_index)
        } else {
            find_paragraph_end(layout, line_index)
        };

        blocks.push(build_semantic_block(layout, line_index, end_line_index)?);
        line_index = end_line_index.saturating_add(1);
    }

    Ok(blocks)
}

fn build_semantic_block(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Result<SemanticBlock> {
    let metadata = resolve_chunk_metadata(layout, start_line_index, end_line_index)?;
    let start_byte = layout
        .lines
        .get(start_line_index)
        .map(|line| line.start_byte)
        .context("missing semantic block start line")?;
    let end_byte = layout
        .lines
        .get(end_line_index)
        .map(|line| line.end_byte)
        .context("missing semantic block end line")?;

    Ok(SemanticBlock {
        start_byte,
        end_byte,
        char_count: semantic_block_char_count(layout, start_line_index, end_line_index),
        heading_path: metadata.heading_path,
    })
}

fn semantic_block_char_count(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> usize {
    layout.lines[start_line_index..=end_line_index]
        .iter()
        .map(|line| {
            line.content.chars().count().saturating_add(
                line.end_byte
                    .saturating_sub(line.start_byte)
                    .saturating_sub(line.content.len()),
            )
        })
        .sum()
}

fn pack_markdown_blocks(
    text: &str,
    blocks: &[SemanticBlock],
    target_chars: usize,
    hard_max_chars: usize,
    overlap_chars: usize,
) -> Result<Vec<ChunkByteRange>> {
    let mut ranges = Vec::new();
    let mut start_index = 0usize;

    while start_index < blocks.len() {
        let block = &blocks[start_index];
        if block.char_count > hard_max_chars {
            ranges.extend(split_oversized_markdown_block(
                text,
                block,
                hard_max_chars,
                overlap_chars,
            )?);
            start_index += 1;
            continue;
        }

        let mut end_index = start_index;
        let heading_path = &block.heading_path;
        while end_index + 1 < blocks.len() {
            let next_block = &blocks[end_index + 1];
            if next_block.char_count > hard_max_chars || next_block.heading_path != *heading_path {
                break;
            }

            let candidate_chars =
                markdown_range_char_count(text, blocks, start_index, end_index + 1);
            if candidate_chars > hard_max_chars {
                break;
            }

            end_index += 1;
            if candidate_chars >= target_chars {
                break;
            }
        }

        ranges.push(ChunkByteRange {
            start_byte: blocks[start_index].start_byte,
            end_byte: blocks[end_index].end_byte,
        });

        if end_index + 1 >= blocks.len() {
            break;
        }

        let next_index = end_index + 1;
        if blocks[next_index].heading_path == *heading_path {
            start_index =
                markdown_overlap_start_index(text, blocks, start_index, end_index, overlap_chars);
        } else {
            start_index = next_index;
        }
    }

    Ok(ranges)
}

fn split_oversized_markdown_block(
    text: &str,
    block: &SemanticBlock,
    hard_max_chars: usize,
    overlap_chars: usize,
) -> Result<Vec<ChunkByteRange>> {
    let block_text = &text[block.start_byte..block.end_byte];
    let splitter = MarkdownSplitter::new(build_chunk_config(hard_max_chars, overlap_chars)?);

    Ok(splitter
        .chunk_char_indices(block_text)
        .filter(|chunk| !chunk.chunk.is_empty())
        .map(|chunk| ChunkByteRange {
            start_byte: block.start_byte.saturating_add(chunk.byte_offset),
            end_byte: block
                .start_byte
                .saturating_add(chunk.byte_offset)
                .saturating_add(chunk.chunk.len()),
        })
        .collect())
}

fn markdown_overlap_start_index(
    text: &str,
    blocks: &[SemanticBlock],
    current_start: usize,
    current_end: usize,
    overlap_chars: usize,
) -> usize {
    if overlap_chars == 0 {
        return current_end.saturating_add(1);
    }

    let mut overlap_start = current_end;
    while overlap_start > current_start
        && blocks[overlap_start - 1].heading_path == blocks[current_end].heading_path
    {
        let overlap_char_count = markdown_byte_range_char_count(
            text,
            blocks[overlap_start - 1].start_byte,
            blocks[current_end].end_byte,
        );
        overlap_start -= 1;
        if overlap_char_count >= overlap_chars {
            break;
        }
    }

    overlap_start.max(current_start.saturating_add(1))
}

fn markdown_range_char_count(
    text: &str,
    blocks: &[SemanticBlock],
    start_index: usize,
    end_index: usize,
) -> usize {
    markdown_byte_range_char_count(
        text,
        blocks[start_index].start_byte,
        blocks[end_index].end_byte,
    )
}

fn markdown_byte_range_char_count(text: &str, start_byte: usize, end_byte: usize) -> usize {
    text[start_byte..end_byte].chars().count()
}

fn find_markdown_fence_end(
    layout: &TextLayout,
    start_line_index: usize,
    fence: MarkdownFence,
) -> usize {
    for line_index in start_line_index.saturating_add(1)..layout.lines.len() {
        if is_markdown_fence_close(layout.lines[line_index].content.trim(), fence) {
            return line_index;
        }
    }

    layout.lines.len().saturating_sub(1)
}

fn find_list_item_end(layout: &TextLayout, start_line_index: usize) -> usize {
    let mut end_line_index = start_line_index;

    for line_index in start_line_index.saturating_add(1)..layout.lines.len() {
        let trimmed = layout.lines[line_index].content.trim();
        if trimmed.is_empty()
            || parse_atx_heading(trimmed).is_some()
            || parse_setext_heading(&layout.lines, line_index).is_some()
            || parse_markdown_fence(trimmed).is_some()
            || parse_markdown_list_item(trimmed).is_some()
        {
            break;
        }
        end_line_index = line_index;
    }

    end_line_index
}

fn find_paragraph_end(layout: &TextLayout, start_line_index: usize) -> usize {
    let mut end_line_index = start_line_index;

    for line_index in start_line_index.saturating_add(1)..layout.lines.len() {
        let trimmed = layout.lines[line_index].content.trim();
        if trimmed.is_empty()
            || parse_atx_heading(trimmed).is_some()
            || parse_setext_heading(&layout.lines, line_index).is_some()
            || parse_markdown_fence(trimmed).is_some()
            || parse_markdown_list_item(trimmed).is_some()
        {
            break;
        }
        end_line_index = line_index;
    }

    end_line_index
}

fn parse_markdown_list_item(trimmed_line: &str) -> Option<()> {
    if ["- ", "* ", "+ "]
        .iter()
        .any(|marker| trimmed_line.starts_with(marker))
    {
        return Some(());
    }

    let digit_count = trimmed_line
        .chars()
        .take_while(|char| char.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let rest = &trimmed_line[digit_count..];
    (rest.starts_with(". ") || rest.starts_with(") ")).then_some(())
}

pub(super) fn resolve_chunk_metadata(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Result<ChunkMetadata> {
    let metadata_anchor_line_index =
        first_non_empty_line_index(layout, start_line_index, end_line_index)
            .unwrap_or(start_line_index);
    let paragraph_start_line_index = *layout
        .paragraph_start_lines
        .get(metadata_anchor_line_index)
        .context("missing paragraph start line for chunk")?;

    Ok(ChunkMetadata {
        paragraph_start_line_index,
        heading_path: common_heading_path_for_range(
            layout,
            metadata_anchor_line_index,
            end_line_index,
        )
        .context("missing heading path metadata for chunk")?,
    })
}

pub(super) fn build_text_layout(text: &str, is_markdown: bool) -> TextLayout {
    let lines = collect_text_lines(text);
    let paragraph_start_lines = build_paragraph_start_lines(&lines);
    let heading_path_by_line = if is_markdown {
        build_markdown_heading_paths(&lines)
    } else {
        vec![Vec::new(); lines.len()]
    };

    TextLayout {
        lines,
        paragraph_start_lines,
        heading_path_by_line,
    }
}

fn collect_text_lines(text: &str) -> Vec<TextLine> {
    if text.is_empty() {
        return vec![TextLine {
            start_byte: 0,
            end_byte: 0,
            content: String::new(),
        }];
    }

    let mut lines = Vec::new();
    let mut start_byte = 0usize;
    for segment in text.split_inclusive('\n') {
        let end_byte = start_byte + segment.len();
        lines.push(TextLine {
            start_byte,
            end_byte,
            content: segment.trim_end_matches(['\r', '\n']).to_string(),
        });
        start_byte = end_byte;
    }

    if !text.ends_with('\n') {
        return lines;
    }

    lines
}

fn build_paragraph_start_lines(lines: &[TextLine]) -> Vec<usize> {
    let mut paragraph_start_lines = Vec::with_capacity(lines.len());
    let mut current_start = 0usize;
    let mut in_paragraph = false;

    for (index, line) in lines.iter().enumerate() {
        if line.content.trim().is_empty() {
            paragraph_start_lines.push(index);
            in_paragraph = false;
            current_start = index.saturating_add(1);
            continue;
        }

        if !in_paragraph {
            current_start = index;
            in_paragraph = true;
        }
        paragraph_start_lines.push(current_start);
    }

    paragraph_start_lines
}

fn build_markdown_heading_paths(lines: &[TextLine]) -> Vec<Vec<String>> {
    let mut heading_path_by_line = Vec::with_capacity(lines.len());
    let mut heading_stack: Vec<String> = Vec::new();
    let mut active_fence: Option<MarkdownFence> = None;

    for index in 0..lines.len() {
        let trimmed = lines[index].content.trim();
        let previous_fence = active_fence;
        if let Some(fence) = parse_markdown_fence(trimmed) {
            if let Some(current_fence) = active_fence {
                if is_markdown_fence_close(trimmed, current_fence) {
                    active_fence = None;
                    heading_path_by_line.push(heading_stack.clone());
                    continue;
                }
            } else {
                active_fence = Some(fence);
                heading_path_by_line.push(heading_stack.clone());
                continue;
            }
        }

        if previous_fence.is_some() {
            heading_path_by_line.push(heading_stack.clone());
            continue;
        }

        let heading = parse_atx_heading(trimmed).or_else(|| parse_setext_heading(lines, index));
        if let Some((level, title)) = heading {
            update_heading_stack(&mut heading_stack, level, title);
        }

        heading_path_by_line.push(heading_stack.clone());
    }

    heading_path_by_line
}

fn parse_atx_heading(trimmed_line: &str) -> Option<(usize, String)> {
    let hashes = trimmed_line.chars().take_while(|char| *char == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }

    let rest = trimmed_line[hashes..].trim();
    if rest.is_empty() {
        return None;
    }

    Some((hashes, rest.trim_end_matches('#').trim().to_string()))
}

fn parse_setext_heading(lines: &[TextLine], index: usize) -> Option<(usize, String)> {
    let title = lines.get(index)?.content.trim();
    if title.is_empty() {
        return None;
    }

    let underline = lines.get(index + 1)?.content.trim();
    if underline.len() < 3 {
        return None;
    }

    if underline.chars().all(|char| char == '=') {
        return Some((1, title.to_string()));
    }
    if underline.chars().all(|char| char == '-') {
        return Some((2, title.to_string()));
    }

    None
}

fn update_heading_stack(stack: &mut Vec<String>, level: usize, title: String) {
    let keep = level.saturating_sub(1);
    stack.truncate(keep);
    stack.push(title);
}

fn parse_markdown_fence(trimmed_line: &str) -> Option<MarkdownFence> {
    let marker = trimmed_line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }

    let length = trimmed_line
        .chars()
        .take_while(|char| *char == marker)
        .count();
    (length >= 3).then_some(MarkdownFence { marker, length })
}

fn is_markdown_fence_close(trimmed_line: &str, active_fence: MarkdownFence) -> bool {
    let Some(fence) = parse_markdown_fence(trimmed_line) else {
        return false;
    };
    if fence.marker != active_fence.marker || fence.length < active_fence.length {
        return false;
    }

    trimmed_line[fence.length..].trim().is_empty()
}

fn first_non_empty_line_index(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Option<usize> {
    (start_line_index..=end_line_index).find(|index| {
        layout
            .lines
            .get(*index)
            .map(|line| !line.content.trim().is_empty())
            .unwrap_or(false)
    })
}

fn common_heading_path_for_range(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Option<Vec<String>> {
    let mut heading_paths = (start_line_index..=end_line_index).filter_map(|index| {
        let line = layout.lines.get(index)?;
        if line.content.trim().is_empty() {
            return None;
        }
        layout.heading_path_by_line.get(index).cloned()
    });
    let mut prefix = heading_paths.next()?;

    for path in heading_paths {
        let shared_len = prefix
            .iter()
            .zip(path.iter())
            .take_while(|(left, right)| left == right)
            .count();
        prefix.truncate(shared_len);
        if prefix.is_empty() {
            break;
        }
    }

    Some(prefix)
}

fn line_index_for_offset(layout: &TextLayout, offset: usize) -> usize {
    let clamped_offset = offset.min(
        layout
            .lines
            .last()
            .map(|line| line.end_byte.saturating_sub(1))
            .unwrap_or_default(),
    );
    let insertion_index = layout
        .lines
        .partition_point(|line| line.start_byte <= clamped_offset);
    insertion_index.saturating_sub(1)
}

fn chunk_reuse_key(text: &str, heading_path: &[String]) -> String {
    let heading_key = heading_path.join("\u{1f}");
    format!("{:x}", md5::compute(format!("{heading_key}\u{0}{text}")))
}
