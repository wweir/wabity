use agent_client_protocol as acp;
use anyhow::Result;

use crate::domain::acp::{
    AcpConfigOption, AcpConfigOptionGroup, AcpConfigOptionKind, AcpConfigValueOption,
    AcpMcpServerConfig, AcpModeOption, AcpSessionRuntimeState,
};

pub(super) fn runtime_state_from_parts(
    modes: Option<acp::SessionModeState>,
    config_options: Option<Vec<acp::SessionConfigOption>>,
) -> AcpSessionRuntimeState {
    let (current_mode_id, available_modes) = modes
        .map(|modes| {
            (
                Some(modes.current_mode_id.to_string()),
                modes.available_modes.iter().map(map_mode_option).collect(),
            )
        })
        .unwrap_or_default();

    AcpSessionRuntimeState {
        current_mode_id,
        available_modes,
        config_options: config_options
            .as_ref()
            .map(|options| map_config_options(options))
            .unwrap_or_default(),
    }
}

pub(super) fn map_config_options(options: &[acp::SessionConfigOption]) -> Vec<AcpConfigOption> {
    options.iter().filter_map(map_config_option).collect()
}

pub(super) fn config_option_contains_value(option: &AcpConfigOption, value_id: &str) -> bool {
    match &option.kind {
        AcpConfigOptionKind::Select {
            options, groups, ..
        } => {
            options.iter().any(|option| option.value_id == value_id)
                || groups.iter().any(|group| {
                    group
                        .options
                        .iter()
                        .any(|option| option.value_id == value_id)
                })
        }
    }
}

pub(super) fn validate_mcp_server_capabilities(
    servers: &[AcpMcpServerConfig],
    capabilities: &acp::McpCapabilities,
) -> Result<()> {
    for server in servers {
        match server {
            AcpMcpServerConfig::Stdio(_) => {}
            AcpMcpServerConfig::Http(server) => {
                if !capabilities.http {
                    anyhow::bail!(
                        "agent 不支持 MCP HTTP transport，但 session 配置了 HTTP server：{}",
                        server.name
                    );
                }
            }
            AcpMcpServerConfig::Sse(server) => {
                if !capabilities.sse {
                    anyhow::bail!(
                        "agent 不支持 MCP SSE transport，但 session 配置了 SSE server：{}",
                        server.name
                    );
                }
            }
        }
    }

    Ok(())
}

pub(super) fn build_mcp_servers(servers: &[AcpMcpServerConfig]) -> Vec<acp::McpServer> {
    servers
        .iter()
        .map(|server| match server {
            AcpMcpServerConfig::Stdio(server) => acp::McpServer::Stdio(
                acp::McpServerStdio::new(&server.name, &server.command)
                    .args(server.args.clone())
                    .env(
                        server
                            .env
                            .iter()
                            .map(|pair| acp::EnvVariable::new(&pair.name, &pair.value))
                            .collect(),
                    ),
            ),
            AcpMcpServerConfig::Http(server) => acp::McpServer::Http(
                acp::McpServerHttp::new(&server.name, &server.url).headers(
                    server
                        .headers
                        .iter()
                        .map(|pair| acp::HttpHeader::new(&pair.name, &pair.value))
                        .collect(),
                ),
            ),
            AcpMcpServerConfig::Sse(server) => acp::McpServer::Sse(
                acp::McpServerSse::new(&server.name, &server.url).headers(
                    server
                        .headers
                        .iter()
                        .map(|pair| acp::HttpHeader::new(&pair.name, &pair.value))
                        .collect(),
                ),
            ),
        })
        .collect()
}

fn map_mode_option(mode: &acp::SessionMode) -> AcpModeOption {
    AcpModeOption {
        id: mode.id.to_string(),
        name: mode.name.clone(),
        description: mode.description.clone(),
    }
}

#[allow(unreachable_patterns)]
fn map_config_option(option: &acp::SessionConfigOption) -> Option<AcpConfigOption> {
    match &option.kind {
        acp::SessionConfigKind::Select(select) => {
            let (options, groups) = match &select.options {
                acp::SessionConfigSelectOptions::Ungrouped(options) => (
                    options.iter().map(map_config_value_option).collect(),
                    Vec::new(),
                ),
                acp::SessionConfigSelectOptions::Grouped(groups) => (
                    Vec::new(),
                    groups.iter().map(map_config_option_group).collect(),
                ),
                _ => (Vec::new(), Vec::new()),
            };

            Some(AcpConfigOption {
                id: option.id.to_string(),
                name: option.name.clone(),
                description: option.description.clone(),
                category: map_config_option_category(option.category.as_ref()),
                kind: AcpConfigOptionKind::Select {
                    current_value_id: select.current_value.to_string(),
                    options,
                    groups,
                },
            })
        }
        _ => {
            tracing::warn!(
                config_option_id = %option.id,
                "ignoring ACP config option kind unsupported by current client mapping"
            );
            None
        }
    }
}

fn map_config_option_group(group: &acp::SessionConfigSelectGroup) -> AcpConfigOptionGroup {
    AcpConfigOptionGroup {
        id: group.group.to_string(),
        name: group.name.clone(),
        options: group.options.iter().map(map_config_value_option).collect(),
    }
}

fn map_config_value_option(option: &acp::SessionConfigSelectOption) -> AcpConfigValueOption {
    AcpConfigValueOption {
        value_id: option.value.to_string(),
        name: option.name.clone(),
        description: option.description.clone(),
    }
}

fn map_config_option_category(
    category: Option<&acp::SessionConfigOptionCategory>,
) -> Option<String> {
    match category {
        Some(acp::SessionConfigOptionCategory::Mode) => Some("mode".to_string()),
        Some(acp::SessionConfigOptionCategory::Model) => Some("model".to_string()),
        Some(acp::SessionConfigOptionCategory::ThoughtLevel) => Some("thought_level".to_string()),
        Some(acp::SessionConfigOptionCategory::Other(value)) => Some(value.clone()),
        Some(_) | None => None,
    }
}
