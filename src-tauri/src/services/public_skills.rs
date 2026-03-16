use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::domain::skills::{
    PublicSkillCatalog, PublicSkillEntry, PublicSkillMeta, PublicSkillMetaEntry, SkillTreeNode,
    SkillTreeNodeKind,
};

const PUBLIC_SKILLS_RELATIVE_PATH: &str = ".agents/skills";

pub struct PublicSkillService;

impl PublicSkillService {
    pub fn load_catalog() -> Result<PublicSkillCatalog> {
        let root_path = public_skills_root()?;
        if !root_path.is_dir() {
            return Ok(PublicSkillCatalog {
                root_path: root_path.to_string_lossy().into_owned(),
                exists: false,
                skills: Vec::new(),
            });
        }

        let mut skill_directories = read_child_directories(&root_path)?;
        skill_directories.sort_by(|left, right| compare_paths(left, right));

        let mut skills = Vec::with_capacity(skill_directories.len());
        for skill_directory in skill_directories {
            skills.push(load_skill_entry(&root_path, &skill_directory)?);
        }

        Ok(PublicSkillCatalog {
            root_path: root_path.to_string_lossy().into_owned(),
            exists: true,
            skills,
        })
    }
}

fn public_skills_root() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().context("failed to determine home directory")?;
    Ok(home_dir.join(PUBLIC_SKILLS_RELATIVE_PATH))
}

fn read_child_directories(root_path: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    let entries = fs::read_dir(root_path).with_context(|| {
        format!(
            "failed to read public skills directory: {}",
            root_path.display()
        )
    })?;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to iterate public skills directory entry under {}",
                root_path.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
        }
    }

    Ok(directories)
}

fn load_skill_entry(root_path: &Path, skill_directory: &Path) -> Result<PublicSkillEntry> {
    let directory_name = skill_directory
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| skill_directory.display().to_string());
    let relative_path = relative_path(root_path, skill_directory);
    let skill_md_path = skill_directory.join("SKILL.md");
    let meta = if skill_md_path.is_file() {
        let content = fs::read_to_string(&skill_md_path).with_context(|| {
            format!("failed to read skill metadata: {}", skill_md_path.display())
        })?;
        parse_skill_meta(&content)
    } else {
        PublicSkillMeta::default()
    };
    let (tree, stats) = build_tree(skill_directory, root_path)?;

    Ok(PublicSkillEntry {
        id: directory_name.clone(),
        directory_name,
        relative_path,
        meta,
        directory_count: stats.directory_count,
        file_count: stats.file_count,
        tree,
    })
}

#[derive(Default)]
struct TreeStats {
    directory_count: usize,
    file_count: usize,
}

fn build_tree(path: &Path, root_path: &Path) -> Result<(SkillTreeNode, TreeStats)> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read path metadata: {}", path.display()))?;
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let relative_path = relative_path(root_path, path);

    if metadata.is_file() {
        return Ok((
            SkillTreeNode {
                name,
                relative_path,
                kind: SkillTreeNodeKind::File,
                children: Vec::new(),
            },
            TreeStats {
                directory_count: 0,
                file_count: 1,
            },
        ));
    }

    let entries = fs::read_dir(path)
        .with_context(|| format!("failed to read skill directory tree: {}", path.display()))?;
    let mut child_paths = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to iterate skill directory tree entry under {}",
                path.display()
            )
        })?;
        let child_path = entry.path();
        if child_path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value == ".DS_Store")
        {
            continue;
        }
        child_paths.push(child_path);
    }
    child_paths.sort_by(|left, right| compare_paths(left, right));

    let mut children = Vec::with_capacity(child_paths.len());
    let mut stats = TreeStats::default();
    for child_path in child_paths {
        let (child_node, child_stats) = build_tree(&child_path, root_path)?;
        if matches!(child_node.kind, SkillTreeNodeKind::Directory) {
            stats.directory_count += 1;
        }
        stats.directory_count += child_stats.directory_count;
        stats.file_count += child_stats.file_count;
        children.push(child_node);
    }

    Ok((
        SkillTreeNode {
            name,
            relative_path,
            kind: SkillTreeNodeKind::Directory,
            children,
        },
        stats,
    ))
}

fn compare_paths(left: &Path, right: &Path) -> Ordering {
    let left_is_dir = left.is_dir();
    let right_is_dir = right.is_dir();
    match right_is_dir.cmp(&left_is_dir) {
        Ordering::Equal => left
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(
                &right
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            ),
        order => order,
    }
}

fn relative_path(root_path: &Path, path: &Path) -> String {
    path.strip_prefix(root_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn parse_skill_meta(content: &str) -> PublicSkillMeta {
    let mut meta = PublicSkillMeta::default();
    let mut lines = content.lines();

    if lines.next().map(str::trim) != Some("---") {
        return meta;
    }

    let mut in_metadata_block = false;
    for line in lines {
        let trimmed_line = line.trim_end();
        let compact = trimmed_line.trim();

        if compact == "---" {
            break;
        }

        if compact.is_empty() {
            continue;
        }

        let indentation = line
            .chars()
            .take_while(|character| character.is_whitespace())
            .count();
        if in_metadata_block && indentation > 0 {
            if let Some((key, value)) = parse_key_value(compact) {
                meta.metadata.push(PublicSkillMetaEntry { key, value });
            }
            continue;
        }

        in_metadata_block = false;
        if compact == "metadata:" {
            in_metadata_block = true;
            continue;
        }

        if let Some((key, value)) = parse_key_value(compact) {
            match key.as_str() {
                "name" => meta.name = Some(value),
                "description" => meta.description = Some(value),
                "argument-hint" => meta.argument_hint = Some(value),
                "license" => meta.license = Some(value),
                _ => {}
            }
        }
    }

    meta
}

fn parse_key_value(line: &str) -> Option<(String, String)> {
    let (key, raw_value) = line.split_once(':')?;
    Some((key.trim().to_string(), normalize_scalar(raw_value.trim())))
}

fn normalize_scalar(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(unquoted) = trimmed
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
    {
        return unquoted.to_string();
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::{build_tree, parse_skill_meta};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parses_skill_frontmatter() {
        let meta = parse_skill_meta(
            r#"---
name: ckm:ui-styling
description: Create beautiful UIs
argument-hint: "[component or layout]"
license: MIT
metadata:
  author: claudekit
  version: "1.0.0"
---

Body
"#,
        );

        assert_eq!(meta.name.as_deref(), Some("ckm:ui-styling"));
        assert_eq!(meta.description.as_deref(), Some("Create beautiful UIs"));
        assert_eq!(meta.argument_hint.as_deref(), Some("[component or layout]"));
        assert_eq!(meta.license.as_deref(), Some("MIT"));
        assert_eq!(meta.metadata.len(), 2);
        assert_eq!(meta.metadata[0].key, "author");
        assert_eq!(meta.metadata[0].value, "claudekit");
        assert_eq!(meta.metadata[1].key, "version");
        assert_eq!(meta.metadata[1].value, "1.0.0");
    }

    #[test]
    fn counts_directories_and_files_in_tree() {
        let root = unique_test_directory();
        let skill_root = root.join("demo-skill");
        let nested_dir = skill_root.join("references");
        fs::create_dir_all(&nested_dir).expect("create test directories");
        fs::write(skill_root.join("SKILL.md"), "---\nname: demo\n---\n").expect("write skill file");
        fs::write(nested_dir.join("guide.md"), "guide").expect("write nested file");

        let (_, stats) = build_tree(&skill_root, &root).expect("build tree");

        assert_eq!(stats.directory_count, 1);
        assert_eq!(stats.file_count, 2);

        fs::remove_dir_all(&root).expect("remove test directory");
    }

    fn unique_test_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("wabity-public-skills-{unique}"))
    }
}
