import type { PublicSkillCatalog, SkillTreeNode } from "../../../lib/tauri/types";
import { getSettingsPanelId, getSettingsTabId } from "../settingsShared";
import type { BindSectionBlockRef } from "../sectionViewShared";

export interface SkillsSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	skillError: string | null;
	skillCatalog: PublicSkillCatalog;
	selectedSkillId: string | null;
	selectedSkill: PublicSkillCatalog["skills"][number] | null;
	onViewSkill: (skillId: string) => void;
}

function renderSkillTreeNode(node: SkillTreeNode) {
	if (node.kind === "file") {
		return (
			<div className="settings-skill-tree-file" key={node.relativePath || node.name}>
				<span className="settings-skill-tree-bullet" aria-hidden="true">
					•
				</span>
				<span>{node.name}</span>
			</div>
		);
	}

	return (
		<details className="settings-skill-tree-directory" key={node.relativePath || node.name} open>
			<summary className="settings-skill-tree-summary">
				<span className="settings-skill-tree-caret" aria-hidden="true">
					▾
				</span>
				<span>{node.name}</span>
			</summary>
			<div className="settings-skill-tree-children">
				{node.children.length > 0 ? (
					node.children.map((child) => renderSkillTreeNode(child))
				) : (
					<span className="settings-help-text settings-help-text-tight">空目录</span>
				)}
			</div>
		</details>
	);
}

export function SkillsSettingsSection({
	bindSectionBlockRef,
	skillError,
	skillCatalog,
	selectedSkillId,
	selectedSkill,
	onViewSkill,
}: SkillsSettingsSectionProps) {
	return (
		<section
			aria-labelledby={getSettingsTabId("skills")}
			className="settings-section"
			id={getSettingsPanelId("skills")}
			role="tabpanel"
			tabIndex={0}
		>
			{skillError ? (
				<div className="settings-banner settings-banner-error">{skillError}</div>
			) : null}

			<div className="settings-skills-layout">
				<div
					className="settings-editor-card settings-editor-card-subtle settings-skill-catalog-panel"
					id="skills-catalog"
					ref={bindSectionBlockRef("skills-catalog")}
				>
					<div className="settings-editor-card-header">
						<div className="settings-acp-sidebar-copy">
							<span className="settings-section-kicker">公共目录</span>
							<strong className="settings-agent-mcp-title">
								{skillCatalog.skills.length} 个 skill
							</strong>
						</div>
					</div>
					<p className="settings-help-text settings-help-text-tight">
						当前只读扫描 <code className="settings-inline-code">{skillCatalog.rootPath}</code>
						，先在上面选一个 skill，再看下面的详情。
					</p>

					{!skillCatalog.exists ? (
						<div className="settings-empty-panel settings-empty-panel-subtle">
							<strong className="settings-empty-title">目录不存在</strong>
							<span className="settings-help-text settings-help-text-tight">
								没找到公共 skill 目录。当前只读取 `~/.agents/skills`。
							</span>
						</div>
					) : skillCatalog.skills.length === 0 ? (
						<div className="settings-empty-panel settings-empty-panel-subtle">
							<strong className="settings-empty-title">没有公共 skill</strong>
							<span className="settings-help-text settings-help-text-tight">
								目录存在，但下面没有可展示的 skill 子目录。
							</span>
						</div>
					) : (
						<div className="settings-catalog-grid settings-catalog-grid-3">
							{skillCatalog.skills.map((skill) => {
								const isSelected = selectedSkillId === skill.id;
								const skillTitle = skill.meta.name?.trim() || skill.directoryName;
								return (
									<article
										className={`settings-agent-list-item settings-skill-list-item ${
											isSelected ? "settings-agent-list-item-selected" : ""
										}`}
										key={skill.id}
									>
										<button
											className="settings-agent-list-item-main settings-skill-list-item-main"
											onClick={() => onViewSkill(skill.id)}
											type="button"
										>
											<div className="settings-agent-title-row">
												<strong className="settings-agent-name">{skillTitle}</strong>
											</div>
											<span className="settings-agent-command-preview">{skill.relativePath}</span>
											<div className="settings-mcp-list-item-badges">
												<span className="settings-status-chip">{skill.directoryCount} 目录</span>
												<span className="settings-status-chip">{skill.fileCount} 文件</span>
											</div>
										</button>
									</article>
								);
							})}
						</div>
					)}
				</div>

				<div
					className="settings-acp-detail"
					id="skills-detail"
					ref={bindSectionBlockRef("skills-detail")}
				>
					{selectedSkill ? (
						<>
							<div className="settings-acp-detail-header">
								<div className="settings-acp-detail-copy">
									<span className="settings-section-kicker">当前 Skill</span>
									<h3 className="settings-subsection-title">
										{selectedSkill.meta.name?.trim() || selectedSkill.directoryName}
									</h3>
								</div>
							</div>

							<div className="settings-editor-card">
								<div className="settings-editor-card-header">
									<strong className="settings-agent-mcp-title">Meta 信息</strong>
									<span className="settings-agent-meta">
										{selectedSkill.directoryCount} 个目录 · {selectedSkill.fileCount} 个文件
									</span>
								</div>
								<div className="settings-skill-table-wrap">
									<table className="settings-skill-table">
										<tbody>
											<tr>
												<th scope="row">目录</th>
												<td>
													<code className="settings-inline-code">{selectedSkill.relativePath}</code>
												</td>
											</tr>
											<tr>
												<th scope="row">名称</th>
												<td>{selectedSkill.meta.name ?? "未声明"}</td>
											</tr>
											<tr>
												<th scope="row">描述</th>
												<td>{selectedSkill.meta.description ?? "未声明"}</td>
											</tr>
											<tr>
												<th scope="row">参数提示</th>
												<td>{selectedSkill.meta.argumentHint ?? "未声明"}</td>
											</tr>
											<tr>
												<th scope="row">License</th>
												<td>{selectedSkill.meta.license ?? "未声明"}</td>
											</tr>
											<tr>
												<th scope="row">目录数</th>
												<td>{selectedSkill.directoryCount}</td>
											</tr>
											<tr>
												<th scope="row">文件数</th>
												<td>{selectedSkill.fileCount}</td>
											</tr>
										</tbody>
									</table>
								</div>
								<div className="settings-editor-card-header">
									<strong className="settings-agent-mcp-title">metadata</strong>
									<span className="settings-agent-meta">
										{selectedSkill.meta.metadata.length > 0
											? `${selectedSkill.meta.metadata.length} 项`
											: "未声明"}
									</span>
								</div>
								{selectedSkill.meta.metadata.length > 0 ? (
									<div className="settings-skill-table-wrap">
										<table className="settings-skill-table">
											<thead>
												<tr>
													<th scope="col">Key</th>
													<th scope="col">Value</th>
												</tr>
											</thead>
											<tbody>
												{selectedSkill.meta.metadata.map((entry) => (
													<tr key={`${entry.key}:${entry.value}`}>
														<td>{entry.key}</td>
														<td>{entry.value}</td>
													</tr>
												))}
											</tbody>
										</table>
									</div>
								) : (
									<span className="settings-help-text settings-help-text-tight">
										`metadata` 段为空或未声明。
									</span>
								)}
							</div>

							<div className="settings-editor-card">
								<div className="settings-editor-card-header">
									<strong className="settings-agent-mcp-title">目录树</strong>
									<span className="settings-agent-meta">只展示名称与层级，不读取文件内容</span>
								</div>
								<div className="settings-skill-tree-panel">
									{renderSkillTreeNode(selectedSkill.tree)}
								</div>
							</div>
						</>
					) : (
						<div className="settings-empty-panel">
							<strong className="settings-empty-title">没有选中的 skill</strong>
							<span className="settings-help-text settings-help-text-tight">
								先从上面的卡片里选择一个公共 skill，下面才会显示 meta 和目录树。
							</span>
						</div>
					)}
				</div>
			</div>
		</section>
	);
}
