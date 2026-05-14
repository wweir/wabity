use super::*;

#[test]
fn ignore_glob_matches_relative_path() {
    let matcher = build_ignore_glob_set(&["**/*.lock".to_string()])
        .expect("glob compilation should succeed")
        .expect("glob set should exist");
    let root = PathBuf::from("/tmp/workspace");
    let path = root.join("Cargo.lock");

    assert!(should_skip_path(&root, &path, Some(&matcher)));
}

#[test]
fn source_root_prefers_deepest_match() {
    let roots = vec![
        PathBuf::from("/tmp/workspace"),
        PathBuf::from("/tmp/workspace/nested"),
    ];
    let path = PathBuf::from("/tmp/workspace/nested/file.txt");

    assert_eq!(
        resolve_source_root_for_path(&roots, &path),
        Some(&PathBuf::from("/tmp/workspace/nested"))
    );
}

#[test]
fn collect_chunks_for_path_splits_text_and_preserves_metadata() {
    let root = temp_test_root("split");
    let file_path = root.join("notes.txt");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, "alpha beta gamma ".repeat(120)).expect("write rag source file");

    let resolved = test_resolved_config(&root);

    let chunks =
        collect_chunks_for_path(&resolved, &file_path).expect("collecting chunks should succeed");

    assert!(chunks.len() > 1);
    assert_eq!(chunks[0].source_root, root.to_string_lossy());
    assert_eq!(chunks[0].absolute_path, file_path.to_string_lossy());
    assert!(chunks[0].line_start.unwrap_or_default() >= 1);
    assert!(chunks[0].line_end.unwrap_or_default() >= chunks[0].line_start.unwrap_or_default());
    assert!(chunks[0].paragraph_line_start.unwrap_or_default() >= 1);
    assert!(chunks.iter().all(|chunk| !chunk.text.trim().is_empty()));
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.chunk_index)
            .collect::<Vec<_>>(),
        (0..chunks.len() as i32).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn supported_rag_document_extensions_are_case_insensitive() {
    assert!(is_supported_document_file(Path::new("/tmp/README.MD")));
    assert!(is_supported_document_file(Path::new("/tmp/notes.mdx")));
    assert!(is_supported_document_file(Path::new("/tmp/plain.txt")));
    assert!(is_supported_document_file(Path::new("/tmp/spec.DOCX")));
    assert!(!is_supported_document_file(Path::new("/tmp/config.toml")));
    assert!(!is_supported_document_file(Path::new("/tmp/README")));
}

#[test]
fn markdown_files_are_packed_by_semantic_blocks() {
    let text = "# Heading\n\n- First note.\n\n- Second note.\n\n- Third note.\n\n## Next\n\nParagraph two.";
    let chunks = split_test_text_for_path(Path::new("/tmp/readme.md"), text, 24, 0)
        .expect("markdown split should succeed");

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].heading_path, vec!["Heading".to_string()]);
    assert!(chunks[0].text.contains("First note."));
    assert!(chunks[0].text.contains("Second note."));
    assert!(chunks[0].text.contains("Third note."));
    assert_eq!(
        chunks[1].heading_path,
        vec!["Heading".to_string(), "Next".to_string()]
    );
    assert!(chunks[1].text.contains("Paragraph two."));
}

#[test]
fn markdown_heading_paths_ignore_fenced_code_with_info_string() {
    let text = "# Intro\n\n```rust\n# not a heading\n```\n\n## Details\n";
    let layout = build_text_layout(text, true);

    assert_eq!(layout.heading_path_by_line[2], vec!["Intro".to_string()]);
    assert_eq!(layout.heading_path_by_line[3], vec!["Intro".to_string()]);
    assert_eq!(
        layout.heading_path_by_line[6],
        vec!["Intro".to_string(), "Details".to_string()]
    );
}

#[test]
fn resolve_chunk_metadata_uses_common_heading_prefix_across_subsections() {
    let text = "# Intro\n\nAlpha\n\n## Details\n\nBeta";
    let layout = build_text_layout(text, true);

    let metadata = resolve_chunk_metadata(&layout, 2, 6).expect("chunk metadata should resolve");

    assert_eq!(metadata.heading_path, vec!["Intro".to_string()]);
}

#[test]
fn resolve_chunk_metadata_anchors_to_first_non_empty_line() {
    let text = "\n\nAlpha\nBeta";
    let layout = build_text_layout(text, false);

    let metadata = resolve_chunk_metadata(&layout, 0, 3).expect("chunk metadata should resolve");

    assert_eq!(metadata.paragraph_start_line_index, 2);
    assert!(metadata.heading_path.is_empty());
}

#[test]
fn plain_text_files_route_to_text_splitter() {
    let text = "# Heading\n\nParagraph one.\n\n## Next\n\nParagraph two.";
    let expected = TextSplitter::new(build_chunk_config(24, 0).expect("valid config"))
        .chunks(text)
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let chunks = split_test_text_for_path(Path::new("/tmp/readme.txt"), text, 24, 0)
        .expect("plain text split should succeed");

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(chunks.iter().all(|chunk| chunk.heading_path.is_empty()));
}

#[test]
fn oversized_markdown_blocks_fall_back_to_markdown_splitter() {
    let text = format!("# Heading\n\n- {}\n", "alpha ".repeat(180));
    let chunks = split_test_text_for_path(Path::new("/tmp/readme.md"), &text, 24, 0)
        .expect("markdown split should succeed");

    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|chunk| {
        chunk.heading_path == vec!["Heading".to_string()]
            && chunk.text.chars().count() <= MARKDOWN_CHUNK_HARD_MAX_CHARS
    }));
}

#[test]
fn collect_chunks_for_path_skips_unsupported_extension() {
    let root = temp_test_root("unsupported-extension");
    let file_path = root.join("notes.toml");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, "title = \"not indexed\"").expect("write rag source file");

    let resolved = test_resolved_config(&root);

    let chunks =
        collect_chunks_for_path(&resolved, &file_path).expect("collecting chunks should succeed");

    assert!(chunks.is_empty());

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn collect_chunks_for_path_supports_text_files_under_plain_text_limit() {
    let root = temp_test_root("large-file");
    let file_path = root.join("large.txt");
    let content = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n".repeat(20_000);
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, &content).expect("write rag source file");

    let resolved = test_resolved_config(&root);

    let file_size = std::fs::metadata(&file_path).expect("read metadata").len();
    assert!(file_size > 1_000_000);
    assert!(file_size < MAX_TEXT_FILE_BYTES_PLAIN_TEXT);

    let chunks = collect_chunks_for_path(&resolved, &file_path)
        .expect("collecting chunks should succeed for large files");

    assert!(!chunks.is_empty());
    assert!(chunks.len() > 1);

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn collect_chunks_for_path_supports_docx_files() {
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
              </w:body>
            </w:document>
        "#;
    let root = temp_test_root("docx-file");
    let file_path = root.join("notes.docx");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, build_test_docx(document_xml, None)).expect("write docx");

    let resolved = test_resolved_config(&root);
    let chunks =
        collect_chunks_for_path(&resolved, &file_path).expect("collect docx chunks should work");

    assert!(!chunks.is_empty());
    assert!(chunks
        .iter()
        .any(|chunk| chunk.heading_path == vec!["Architecture".to_string()]));
    assert!(chunks
        .iter()
        .any(|chunk| chunk.text.contains("Alpha paragraph.")));

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn collect_chunks_for_path_skips_text_files_over_plain_text_limit() {
    let root = temp_test_root("too-large-file");
    let file_path = root.join("too-large.txt");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    let file = std::fs::File::create(&file_path).expect("create oversized rag source file");
    file.set_len(MAX_TEXT_FILE_BYTES_PLAIN_TEXT + 1)
        .expect("set oversized rag source length");

    let resolved = test_resolved_config(&root);

    let chunks = collect_chunks_for_path(&resolved, &file_path)
        .expect("collecting chunks should skip oversized files");

    assert!(chunks.is_empty());

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}
