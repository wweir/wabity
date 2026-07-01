use std::collections::HashSet;

use anyhow::{Context, Result};

use crate::{
    domain::acp::{
        AcpAgentCatalog, AcpAgentConfig, AcpMcpServerCatalog, AcpMcpServerConfig,
        AcpMcpServerHttpConfig, AcpMcpServerSseConfig, AcpMcpServerStdioConfig, AcpNameValuePair,
        BuiltinMcpConfig,
    },
    infrastructure::config::{
        acp_agent::{make_agent_id, normalize_acp_agent_command},
        SavedAcpSession,
    },
    services::builtin_mcp,
};

pub(super) fn normalize_acp_agent_catalog(catalog: AcpAgentCatalog) -> Result<AcpAgentCatalog> {
    let mut agents = Vec::with_capacity(catalog.agents.len());
    let mut used_ids = HashSet::new();

    for (index, agent) in catalog.agents.into_iter().enumerate() {
        let name = agent.name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("第 {} 个 ACP agent 名称为空", index + 1);
        }

        let mut program = agent.program;
        let mut args = agent.args;
        let mut shell_command = agent.shell_command;
        let mut launch_mode = agent.launch_mode;
        normalize_acp_agent_command(
            &mut program,
            &mut args,
            &mut shell_command,
            &mut launch_mode,
            &format!("第 {} 个 ACP agent", index + 1),
        )?;

        let id = make_agent_id(agent.id.trim(), index, &used_ids);
        agents.push(AcpAgentConfig {
            id: id.clone(),
            name,
            program,
            args,
            shell_command,
            launch_mode,
            mcp_servers: Vec::new(),
        });
        used_ids.insert(id);
    }

    let default_agent_id = if agents.is_empty() {
        None
    } else {
        let requested_default = catalog
            .default_agent_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .filter(|default_agent_id| agents.iter().any(|agent| agent.id == *default_agent_id));
        requested_default.or_else(|| agents.first().map(|agent| agent.id.clone()))
    };

    Ok(AcpAgentCatalog {
        agents,
        default_agent_id,
    })
}

pub(super) fn normalize_acp_mcp_server_catalog(
    catalog: AcpMcpServerCatalog,
) -> Result<AcpMcpServerCatalog> {
    Ok(AcpMcpServerCatalog {
        servers: normalize_mcp_servers(catalog.servers, "Agent MCP 服务")?,
        builtin: normalize_builtin_mcp_config(catalog.builtin),
    })
}

pub(super) fn normalize_builtin_mcp_config(config: BuiltinMcpConfig) -> BuiltinMcpConfig {
    let mut enabled_modules = config.enabled_modules;
    enabled_modules.sort();
    enabled_modules.dedup();

    BuiltinMcpConfig {
        enabled: config.enabled,
        enabled_modules,
    }
}

pub(super) fn remove_legacy_builtin_mcp_server_from_snapshot(
    mut snapshot: SavedAcpSession,
) -> SavedAcpSession {
    snapshot
        .mcp_servers
        .retain(|server| !builtin_mcp::is_builtin_server(server));
    snapshot
}

fn normalize_mcp_servers(
    servers: Vec<AcpMcpServerConfig>,
    owner_label: &str,
) -> Result<Vec<AcpMcpServerConfig>> {
    let mut normalized = Vec::with_capacity(servers.len());

    for (server_index, server) in servers.into_iter().enumerate() {
        let label = format!("{owner_label} 的第 {} 个 MCP server", server_index + 1);
        let server = match server {
            AcpMcpServerConfig::Stdio(config) => {
                let name = config.name.trim().to_string();
                if name.is_empty() {
                    anyhow::bail!("{label} 名称为空");
                }
                let command = config.command.trim().to_string();
                if command.is_empty() {
                    anyhow::bail!("{label} 的 stdio command 为空");
                }

                AcpMcpServerConfig::Stdio(AcpMcpServerStdioConfig {
                    name,
                    command,
                    args: config
                        .args
                        .into_iter()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .collect(),
                    env: normalize_name_value_pairs(config.env, &format!("{label} 的 env"))?,
                })
            }
            AcpMcpServerConfig::Http(config) => {
                let name = config.name.trim().to_string();
                if name.is_empty() {
                    anyhow::bail!("{label} 名称为空");
                }
                let url = normalize_mcp_remote_url(&config.url, &format!("{label} 的 http url"))?;

                AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                    name,
                    url,
                    headers: normalize_name_value_pairs(
                        config.headers,
                        &format!("{label} 的 headers"),
                    )?,
                })
            }
            AcpMcpServerConfig::Sse(config) => {
                let name = config.name.trim().to_string();
                if name.is_empty() {
                    anyhow::bail!("{label} 名称为空");
                }
                let url = normalize_mcp_remote_url(&config.url, &format!("{label} 的 sse url"))?;

                AcpMcpServerConfig::Sse(AcpMcpServerSseConfig {
                    name,
                    url,
                    headers: normalize_name_value_pairs(
                        config.headers,
                        &format!("{label} 的 headers"),
                    )?,
                })
            }
        };

        normalized.push(server);
    }

    Ok(normalized)
}

fn normalize_name_value_pairs(
    pairs: Vec<AcpNameValuePair>,
    label: &str,
) -> Result<Vec<AcpNameValuePair>> {
    let mut normalized = Vec::with_capacity(pairs.len());

    for (index, pair) in pairs.into_iter().enumerate() {
        let name = pair.name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("{label} 的第 {} 项 name 为空", index + 1);
        }

        normalized.push(AcpNameValuePair {
            name,
            value: pair.value.trim().to_string(),
        });
    }

    Ok(normalized)
}

pub(super) fn normalize_mcp_remote_url(url: &str, label: &str) -> Result<String> {
    let normalized = url.trim();
    if normalized.is_empty() {
        anyhow::bail!("{label} 为空");
    }

    let parsed =
        reqwest::Url::parse(normalized).with_context(|| format!("{label} 不是合法 URL"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(normalized.to_string()),
        _ => anyhow::bail!("{label} 只支持 http:// 或 https://"),
    }
}
