use std::ffi::OsString;
use std::process::Stdio;

#[cfg(not(target_os = "windows"))]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{ffi::CStr, os::unix::ffi::OsStringExt};

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::domain::acp::{AcpAgentConfig, AcpAgentLaunchMode};

pub(super) fn build_agent_command(
    agent: &AcpAgentConfig,
    workspace_root: &std::path::Path,
) -> Result<Command> {
    let mut command = match agent.launch_mode {
        AcpAgentLaunchMode::Direct => {
            if agent.program.trim().is_empty() {
                anyhow::bail!("ACP agent program is empty");
            }

            let mut command = Command::new(&agent.program);
            command.args(&agent.args);
            command
        }
        AcpAgentLaunchMode::LoginShell | AcpAgentLaunchMode::InteractiveShell => {
            let shell_command = agent
                .shell_command
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .context("ACP agent shell command is empty")?;
            shell_command_command(shell_command, agent.launch_mode)
        }
    };

    command
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    Ok(command)
}

#[cfg(target_os = "windows")]
fn shell_command_command(shell_command: &str, launch_mode: AcpAgentLaunchMode) -> Command {
    let shell = std::env::var_os("COMSPEC")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("cmd.exe"));
    let mut command = Command::new(shell);
    let shell_flag = match launch_mode {
        AcpAgentLaunchMode::LoginShell => "/C",
        AcpAgentLaunchMode::InteractiveShell => "/C",
        AcpAgentLaunchMode::Direct => "/C",
    };
    command.args([shell_flag, shell_command]);
    command
}

#[cfg(not(target_os = "windows"))]
fn shell_command_command(shell_command: &str, launch_mode: AcpAgentLaunchMode) -> Command {
    let shell = resolve_user_shell();
    let mut command = Command::new(shell);
    match launch_mode {
        AcpAgentLaunchMode::Direct | AcpAgentLaunchMode::LoginShell => {
            command.args(["-l", "-c", shell_command]);
        }
        AcpAgentLaunchMode::InteractiveShell => {
            command.args(["-i", "-l", "-c", shell_command]);
        }
    }
    command
}

#[cfg(not(target_os = "windows"))]
fn resolve_user_shell() -> OsString {
    choose_user_shell(
        resolve_shell_from_passwd().map(PathBuf::into_os_string),
        std::env::var_os("SHELL").filter(|value| !value.is_empty()),
    )
}

#[cfg(not(target_os = "windows"))]
pub(super) fn choose_user_shell(
    passwd_shell: Option<OsString>,
    env_shell: Option<OsString>,
) -> OsString {
    passwd_shell
        .filter(shell_supports_login_shell_flags)
        .or_else(|| env_shell.filter(shell_supports_login_shell_flags))
        .unwrap_or_else(|| OsString::from("/bin/sh"))
}

#[cfg(not(target_os = "windows"))]
fn shell_supports_login_shell_flags(shell: &OsString) -> bool {
    let Some(name) = Path::new(shell)
        .file_name()
        .and_then(|value| value.to_str())
    else {
        return false;
    };

    matches!(
        name.to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "mksh" | "pdksh" | "ash"
    )
}

#[cfg(unix)]
fn resolve_shell_from_passwd() -> Option<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let mut passwd = std::mem::MaybeUninit::<libc::passwd>::zeroed();
    let mut result = std::ptr::null_mut();
    let initial_size = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut buffer = vec![0_u8; initial_size.max(1024) as usize];

    loop {
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                passwd.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };

        if status == 0 {
            break;
        }
        if status == libc::ERANGE {
            buffer.resize(buffer.len().saturating_mul(2).max(1024), 0);
            continue;
        }
        return None;
    }

    if result.is_null() {
        return None;
    }

    let passwd = unsafe { passwd.assume_init() };
    let shell = passwd.pw_shell;
    if shell.is_null() {
        return None;
    }

    let bytes = unsafe { CStr::from_ptr(shell) }.to_bytes();
    if bytes.is_empty() {
        return None;
    }

    Some(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}
