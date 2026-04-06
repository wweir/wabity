use std::collections::HashSet;

use anyhow::{Context, Result};

use crate::domain::acp::AcpAgentLaunchMode;

pub(crate) fn normalize_agent_name(
    name: &str,
    program: &str,
    shell_command: Option<&str>,
) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    derive_agent_name(shell_command, program)
}

pub(crate) fn normalize_acp_agent_command(
    program: &mut String,
    args: &mut Vec<String>,
    shell_command: &mut Option<String>,
    launch_mode: &mut AcpAgentLaunchMode,
    subject: &str,
) -> Result<()> {
    *program = program.trim().to_string();
    *args = args
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    *shell_command = shell_command
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    *launch_mode = normalize_agent_launch_mode(*launch_mode, program, shell_command);
    *launch_mode = normalize_windows_shell_launch_mode(*launch_mode);

    if *launch_mode == AcpAgentLaunchMode::Direct {
        if let Some(command_line) = shell_command.as_deref() {
            let (next_program, next_args) = parse_direct_agent_command(command_line)
                .with_context(|| format!("{subject} 直连命令解析失败"))?;
            *program = next_program;
            *args = next_args;
        } else if program.is_empty() {
            anyhow::bail!("{subject} 命令为空");
        }
    } else if shell_command.is_none() {
        anyhow::bail!("{subject} 选择了 shell 启动模式，但没有提供 shell 命令");
    }

    Ok(())
}

pub(crate) fn parse_direct_agent_command(shell_command: &str) -> Result<(String, Vec<String>)> {
    let parsed = parse_direct_agent_argv(shell_command.trim()).context("命令为空或格式不合法")?;
    let program = parsed
        .first()
        .map(String::as_str)
        .context("命令为空")?
        .trim();
    if program.is_empty() {
        anyhow::bail!("命令为空");
    }

    Ok((program.to_string(), parsed[1..].to_vec()))
}

pub(crate) fn derive_agent_name(shell_command: Option<&str>, program: &str) -> String {
    let command = shell_command
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(program.trim());
    let first_token = command.split_whitespace().next().unwrap_or_default();
    let normalized = first_token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first_token);
    let lowercase = normalized.to_ascii_lowercase();

    if lowercase.contains("opencode") {
        return "OpenCode".to_string();
    }
    if lowercase.contains("claude-agent") {
        return "Claude Agent".to_string();
    }
    if lowercase.contains("codex") {
        return "Codex".to_string();
    }
    if normalized.is_empty() {
        return "ACP Agent".to_string();
    }

    normalized.to_string()
}

pub(crate) fn make_agent_id(seed: &str, index: usize, used_ids: &HashSet<String>) -> String {
    let mut base = seed
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = format!("agent-{}", index + 1);
    }

    let mut candidate = base.clone();
    let mut suffix = 2_u32;
    while used_ids.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix = suffix.saturating_add(1);
    }
    candidate
}

pub(crate) fn normalize_agent_launch_mode(
    launch_mode: AcpAgentLaunchMode,
    program: &str,
    shell_command: &Option<String>,
) -> AcpAgentLaunchMode {
    if shell_command.is_none() && !program.trim().is_empty() {
        return AcpAgentLaunchMode::Direct;
    }

    launch_mode
}

#[cfg(target_os = "windows")]
fn normalize_windows_shell_launch_mode(launch_mode: AcpAgentLaunchMode) -> AcpAgentLaunchMode {
    match launch_mode {
        AcpAgentLaunchMode::InteractiveShell => AcpAgentLaunchMode::LoginShell,
        other => other,
    }
}

#[cfg(not(target_os = "windows"))]
fn normalize_windows_shell_launch_mode(launch_mode: AcpAgentLaunchMode) -> AcpAgentLaunchMode {
    launch_mode
}

fn parse_direct_agent_argv(shell_command: &str) -> Option<Vec<String>> {
    if cfg!(target_os = "windows") {
        parse_windows_command_line(shell_command)
    } else {
        shlex::split(shell_command)
    }
}

pub(super) fn parse_windows_command_line(shell_command: &str) -> Option<Vec<String>> {
    if shell_command.trim().is_empty() {
        return None;
    }

    let mut chars = shell_command.chars().peekable();
    let mut args = Vec::new();

    while chars.peek().is_some() {
        while matches!(chars.peek(), Some(ch) if ch.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let mut arg = String::new();
        let mut in_quotes = false;
        let mut backslashes = 0_usize;

        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    backslashes += 1;
                }
                '"' => {
                    arg.push_str(&"\\".repeat(backslashes / 2));
                    if backslashes.is_multiple_of(2) {
                        if in_quotes && matches!(chars.peek(), Some('"')) {
                            arg.push('"');
                            chars.next();
                        } else {
                            in_quotes = !in_quotes;
                        }
                    } else {
                        arg.push('"');
                    }
                    backslashes = 0;
                }
                _ if ch.is_whitespace() && !in_quotes => {
                    arg.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    break;
                }
                _ => {
                    arg.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    arg.push(ch);
                }
            }
        }

        if in_quotes {
            return None;
        }

        arg.push_str(&"\\".repeat(backslashes));
        args.push(arg);
    }

    if args.is_empty() {
        None
    } else {
        Some(args)
    }
}
