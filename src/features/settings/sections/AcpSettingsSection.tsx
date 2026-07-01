import type { ReactElement } from "react";
import type { BindSectionBlockRef } from "../sectionViewShared";
import { renderDependencyHealthList } from "../sectionViewShared";
import type { DependencyHealthItem } from "../settingsTypes";

export interface AgentRuntimePanelProps {
	bindSectionBlockRef: BindSectionBlockRef;
	healthItems: DependencyHealthItem[];
	blockId?: string;
}

const bridgedProviderLabels = ["OpenAI", "OpenRouter", "DeepSeek", "Ollama", "SiliconFlow"];

export function AgentRuntimePanel({
	bindSectionBlockRef,
	healthItems,
	blockId = "extensions-agent",
}: AgentRuntimePanelProps): ReactElement {
	return (
		<div
			className="settings-editor-card settings-agent-runtime-card"
			id={blockId}
			ref={bindSectionBlockRef(blockId)}
		>
			<div className="settings-agent-runtime-hero">
				<div className="settings-acp-detail-copy">
					<span className="settings-section-kicker">Agent</span>
					<strong className="settings-agent-runtime-title">内嵌 Agent session</strong>
					<span className="settings-help-text settings-help-text-tight">
						创建 session 时只在安全映射时传入文档问答模型，否则沿用底层配置。
					</span>
				</div>
			</div>

			{renderDependencyHealthList(healthItems)}

			<div className="settings-agent-runtime-grid" aria-label="Agent 运行时规则">
				<div className="settings-agent-runtime-row settings-agent-runtime-row-wide">
					<span className="settings-info-label">Wabity 传入项</span>
					<strong>workspace 必传；provider/model/api key 仅在可安全桥接时传入</strong>
					<span className="settings-help-text settings-help-text-tight">
						功能页的文档问答模型只用于初始化 session 默认模型，不会把轻量问答历史或 Wabity MCP
						清单混进 Agent session。
					</span>
				</div>
				<div className="settings-agent-runtime-row settings-agent-runtime-row-wide">
					<span className="settings-info-label">运行时配置</span>
					<strong>模型默认值、thinking、工具策略和重试 / 压缩仍由 Pi 管理</strong>
					<span className="settings-help-text settings-help-text-tight">
						配置位置：<code className="settings-inline-code">~/.pi/agent/settings.json</code>、
						<code className="settings-inline-code">.pi/settings.json</code> 和
						<code className="settings-inline-code">~/.pi/agent/models.json</code>。
					</span>
				</div>
				<div className="settings-agent-runtime-row settings-agent-runtime-row-wide">
					<span className="settings-info-label">可自动桥接 provider</span>
					<div className="settings-agent-provider-list">
						{bridgedProviderLabels.map((label) => (
							<span className="settings-agent-provider-chip" key={label}>
								{label}
							</span>
						))}
					</div>
					<span className="settings-help-text settings-help-text-tight">
						其它 provider 不做猜测映射；请在{" "}
						<code className="settings-inline-code">settings.json</code> 或
						<code className="settings-inline-code">models.json</code> 中配置。
					</span>
				</div>
				<div className="settings-agent-runtime-row">
					<span className="settings-info-label">外部命令</span>
					<strong>不再支持 ACP agent 命令</strong>
					<span className="settings-help-text settings-help-text-tight">
						不展示 codex-acp、claude-agent-acp 或 opencode acp 入口。
					</span>
				</div>
				<div className="settings-agent-runtime-row">
					<span className="settings-info-label">Agent 工具</span>
					<strong>内置工具只注入 Agent session</strong>
					<span className="settings-help-text settings-help-text-tight">
						Wabity 不再对外暴露本地 MCP endpoint，也不把这些工具混入文档问答链路。
					</span>
				</div>
			</div>
		</div>
	);
}
