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
					<span className="settings-section-kicker">Agent runtime</span>
					<strong className="settings-agent-runtime-title">内嵌 Agent session</strong>
					<span className="settings-help-text settings-help-text-tight">
						创建 session 时会优先复用“功能”里的文档问答模型；无法安全映射的 provider 继续交给底层
						SDK 配置处理。
					</span>
				</div>
				<span className="settings-agent-runtime-badge">无需单独配置</span>
			</div>

			{renderDependencyHealthList(healthItems)}

			<div className="settings-agent-runtime-grid" aria-label="Agent 运行时规则">
				<div className="settings-agent-runtime-row settings-agent-runtime-row-wide">
					<span className="settings-info-label">模型来源</span>
					<strong>优先复用文档问答模型</strong>
					<span className="settings-help-text settings-help-text-tight">
						如果没有可用问答模型，会退回到底层 SDK 自身的 provider/model 配置。
					</span>
				</div>
				<div className="settings-agent-runtime-row settings-agent-runtime-row-wide">
					<span className="settings-info-label">自动桥接 provider</span>
					<div className="settings-agent-provider-list">
						{bridgedProviderLabels.map((label) => (
							<span className="settings-agent-provider-chip" key={label}>
								{label}
							</span>
						))}
					</div>
				</div>
				<div className="settings-agent-runtime-row">
					<span className="settings-info-label">外部命令</span>
					<strong>已删除</strong>
				</div>
				<div className="settings-agent-runtime-row">
					<span className="settings-info-label">全局 MCP</span>
					<strong>不自动注入 session</strong>
				</div>
			</div>
		</div>
	);
}
