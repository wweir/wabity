import type { Dispatch, ReactElement, SetStateAction } from "react";
import type { LlmProviderConfig, LlmSettings, PromptsSettings } from "../../../lib/tauri/types";
import {
	DISCARD_DRAFT_BUTTON_LABEL,
	getSettingsPanelId,
	getSettingsTabId,
} from "../settingsShared";
import type { BindSectionBlockRef } from "../sectionViewShared";
import type { SettingsSectionId } from "../settingsTypes";

type LlmProviderDisplaySource = Pick<LlmProviderConfig, "baseUrl" | "models" | "name">;

interface AiTaskPromptCardProps {
	bindSectionBlockRef: BindSectionBlockRef;
	blockId: string;
	description: string;
	eligibleAiTaskProviders: LlmProviderConfig[];
	hasUnsavedChanges: boolean;
	onDiscardDraft: () => void;
	onPromptChange: (value: string) => void;
	onProviderChange: (modelId: string | null) => void;
	onRestoreDefault: () => void;
	onSave: () => void;
	onSelectSection: (sectionId: SettingsSectionId) => void;
	promptExpanded: boolean;
	promptFieldId: string;
	promptHelpText: string;
	promptPanelId: string;
	promptSummary: string;
	promptValue: string;
	providerFieldId: string;
	providerLabel: string;
	saveButtonLabel: string;
	savingAiTaskConfig: boolean;
	savingTaskConfig: boolean;
	selectedProvider: LlmProviderConfig | null;
	summarizeLlmProviderProfile: (provider: LlmProviderConfig) => string;
	title: string;
	togglePromptExpanded: () => void;
	value: string;
}

export interface PromptsSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	llmSettings: LlmSettings;
	promptsSettings: PromptsSettings;
	eligibleAiTaskProviders: LlmProviderConfig[];
	selectedTranslationProvider: LlmProviderConfig | null;
	selectedQuestionAnswerProvider: LlmProviderConfig | null;
	translationPromptExpanded: boolean;
	questionAnswerPromptExpanded: boolean;
	translationPromptSummary: string;
	questionAnswerPromptSummary: string;
	translationHasUnsavedChanges: boolean;
	questionAnswerHasUnsavedChanges: boolean;
	savingAiTaskConfig: boolean;
	savingTranslationConfig: boolean;
	savingQuestionAnswerConfig: boolean;
	setTranslationPromptExpanded: Dispatch<SetStateAction<boolean>>;
	setQuestionAnswerPromptExpanded: Dispatch<SetStateAction<boolean>>;
	setPromptsSettings: Dispatch<SetStateAction<PromptsSettings>>;
	onLlmRouteProviderChange: (
		key: "translationModelId" | "questionAnswerModelId",
		modelId: string | null,
	) => void;
	onSelectSection: (sectionId: SettingsSectionId) => void;
	onDiscardTranslationDraft: () => void;
	onDiscardQuestionAnswerDraft: () => void;
	onSaveTranslationConfig: () => Promise<void>;
	onSaveQuestionAnswerConfig: () => Promise<void>;
	summarizeLlmProviderProfile: (provider: LlmProviderConfig) => string;
	defaultTranslationPrompt: string;
	defaultQuestionAnswerPrompt: string;
}

function getLlmProviderDisplayName(provider: LlmProviderDisplaySource): string {
	return provider.name || provider.models[0]?.model || provider.baseUrl;
}

function AiTaskPromptCard({
	bindSectionBlockRef,
	blockId,
	description,
	eligibleAiTaskProviders,
	hasUnsavedChanges,
	onDiscardDraft,
	onPromptChange,
	onProviderChange,
	onRestoreDefault,
	onSave,
	onSelectSection,
	promptExpanded,
	promptFieldId,
	promptHelpText,
	promptPanelId,
	promptSummary,
	promptValue,
	providerFieldId,
	providerLabel,
	saveButtonLabel,
	savingAiTaskConfig,
	savingTaskConfig,
	selectedProvider,
	summarizeLlmProviderProfile,
	title,
	togglePromptExpanded,
	value,
}: AiTaskPromptCardProps): ReactElement {
	return (
		<article
			className="settings-editor-card settings-task-card"
			id={blockId}
			ref={bindSectionBlockRef(blockId)}
		>
			<div className="settings-task-card-header">
				<div className="settings-acp-detail-copy">
					<div className="settings-task-card-title-row">
						<h3 className="settings-subsection-title">{title}</h3>
						<span
							className={`settings-status-chip ${
								hasUnsavedChanges ? "settings-status-chip-strong" : ""
							}`}
						>
							{hasUnsavedChanges ? "有草稿" : "已同步"}
						</span>
					</div>
					<span className="settings-help-text settings-help-text-tight">{description}</span>
				</div>
			</div>
			<div className="settings-item settings-item-stacked settings-item-wide">
				<label className="settings-label settings-label-stacked" htmlFor={providerFieldId}>
					<span>{providerLabel}</span>
				</label>
				<select
					className="settings-select"
					disabled={savingAiTaskConfig}
					id={providerFieldId}
					onChange={(event) => onProviderChange(event.target.value || null)}
					value={value}
				>
					<option value="">请选择一个普通 LLM 条目</option>
					{eligibleAiTaskProviders.map((provider) => (
						<option
							key={provider.models[0]?.id ?? provider.id}
							value={provider.models[0]?.id ?? ""}
						>
							{getLlmProviderDisplayName(provider)}
							{` · ${summarizeLlmProviderProfile(provider)}`}
						</option>
					))}
				</select>
				<span className="settings-help-text settings-help-text-tight">
					当前条目：
					{selectedProvider
						? `${getLlmProviderDisplayName(selectedProvider)} · ${summarizeLlmProviderProfile(selectedProvider)}`
						: "未配置"}
				</span>
				{eligibleAiTaskProviders.length === 0 ? (
					<span className="settings-help-text settings-help-text-tight">
						当前没有可选普通 LLM 条目。
						<button
							className="settings-text-link"
							onClick={() => onSelectSection("llm")}
							type="button"
						>
							前往模型接入
						</button>
					</span>
				) : null}
			</div>
			<div className="settings-item settings-item-stacked">
				<div className="settings-task-field-header">
					<label className="settings-label settings-label-stacked" htmlFor={promptFieldId}>
						<span>提示词</span>
					</label>
					<button
						aria-controls={promptPanelId}
						aria-expanded={promptExpanded}
						className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-task-toggle-button"
						onClick={togglePromptExpanded}
						type="button"
					>
						{promptExpanded ? "收起编辑" : "展开编辑"}
					</button>
				</div>
				<span className="settings-help-text settings-help-text-tight">{promptSummary}</span>
				{promptExpanded ? (
					<div className="settings-task-prompt-panel" id={promptPanelId}>
						<textarea
							className="settings-textarea settings-task-prompt-textarea"
							disabled={savingAiTaskConfig}
							id={promptFieldId}
							onChange={(event) => onPromptChange(event.target.value)}
							placeholder="留空并保存时会恢复内置默认提示词"
							rows={6}
							value={promptValue}
						/>
						<span className="settings-help-text settings-help-text-tight">{promptHelpText}</span>
					</div>
				) : null}
			</div>
			<div className="settings-task-card-actions">
				<div className="settings-task-card-secondary-actions">
					<button
						className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-task-secondary-action"
						disabled={savingAiTaskConfig}
						onClick={onRestoreDefault}
						type="button"
					>
						恢复默认
					</button>
					<button
						className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-task-secondary-action"
						disabled={savingAiTaskConfig || !hasUnsavedChanges}
						onClick={onDiscardDraft}
						type="button"
					>
						{DISCARD_DRAFT_BUTTON_LABEL}
					</button>
				</div>
				<button
					className="settings-button settings-task-card-save"
					disabled={savingAiTaskConfig || !hasUnsavedChanges}
					onClick={onSave}
					type="button"
				>
					{savingTaskConfig ? "保存中..." : saveButtonLabel}
				</button>
			</div>
		</article>
	);
}

export function PromptsSettingsSection({
	bindSectionBlockRef,
	llmSettings,
	promptsSettings,
	eligibleAiTaskProviders,
	selectedTranslationProvider,
	selectedQuestionAnswerProvider,
	translationPromptExpanded,
	questionAnswerPromptExpanded,
	translationPromptSummary,
	questionAnswerPromptSummary,
	translationHasUnsavedChanges,
	questionAnswerHasUnsavedChanges,
	savingAiTaskConfig,
	savingTranslationConfig,
	savingQuestionAnswerConfig,
	setTranslationPromptExpanded,
	setQuestionAnswerPromptExpanded,
	setPromptsSettings,
	onLlmRouteProviderChange,
	onSelectSection,
	onDiscardTranslationDraft,
	onDiscardQuestionAnswerDraft,
	onSaveTranslationConfig,
	onSaveQuestionAnswerConfig,
	summarizeLlmProviderProfile,
	defaultTranslationPrompt,
	defaultQuestionAnswerPrompt,
}: PromptsSettingsSectionProps) {
	return (
		<section
			aria-labelledby={getSettingsTabId("prompts")}
			className="settings-section"
			id={getSettingsPanelId("prompts")}
			role="tabpanel"
			tabIndex={0}
		>
			<div className="settings-ai-task-grid">
				<AiTaskPromptCard
					bindSectionBlockRef={bindSectionBlockRef}
					blockId="prompts-translation"
					description="选择翻译模型。提示词默认已内置，只有要覆盖时再展开编辑。"
					eligibleAiTaskProviders={eligibleAiTaskProviders}
					hasUnsavedChanges={translationHasUnsavedChanges}
					onDiscardDraft={onDiscardTranslationDraft}
					onPromptChange={(value) =>
						setPromptsSettings((current) => ({
							...current,
							translationPrompt: value,
						}))
					}
					onProviderChange={(modelId) => onLlmRouteProviderChange("translationModelId", modelId)}
					onRestoreDefault={() =>
						setPromptsSettings((current) => ({
							...current,
							translationPrompt: defaultTranslationPrompt,
						}))
					}
					onSave={() => void onSaveTranslationConfig()}
					onSelectSection={onSelectSection}
					promptExpanded={translationPromptExpanded}
					promptFieldId="translation-prompt"
					promptHelpText="默认规则只输出译文，并保留 Markdown、代码块、链接和原文格式。"
					promptPanelId="translation-prompt-panel"
					promptSummary={translationPromptSummary}
					promptValue={promptsSettings.translationPrompt}
					providerFieldId="translation-provider"
					providerLabel="翻译模型"
					saveButtonLabel="保存翻译配置"
					savingAiTaskConfig={savingAiTaskConfig}
					savingTaskConfig={savingTranslationConfig}
					selectedProvider={selectedTranslationProvider}
					summarizeLlmProviderProfile={summarizeLlmProviderProfile}
					title="翻译配置"
					togglePromptExpanded={() => setTranslationPromptExpanded((current) => !current)}
					value={llmSettings.translationModelId ?? ""}
				/>

				<AiTaskPromptCard
					bindSectionBlockRef={bindSectionBlockRef}
					blockId="prompts-rag-answer"
					description="这里只控制回答阶段。文档检索仍然使用 RAG 页里的 Embedding。"
					eligibleAiTaskProviders={eligibleAiTaskProviders}
					hasUnsavedChanges={questionAnswerHasUnsavedChanges}
					onDiscardDraft={onDiscardQuestionAnswerDraft}
					onPromptChange={(value) =>
						setPromptsSettings((current) => ({
							...current,
							ragAnswerSystemPrompt: value,
						}))
					}
					onProviderChange={(modelId) => onLlmRouteProviderChange("questionAnswerModelId", modelId)}
					onRestoreDefault={() =>
						setPromptsSettings((current) => ({
							...current,
							ragAnswerSystemPrompt: defaultQuestionAnswerPrompt,
						}))
					}
					onSave={() => void onSaveQuestionAnswerConfig()}
					onSelectSection={onSelectSection}
					promptExpanded={questionAnswerPromptExpanded}
					promptFieldId="rag-answer-system-prompt"
					promptHelpText="默认规则要求先取证，再区分事实与推断；证据不够时直接说明。"
					promptPanelId="question-answer-prompt-panel"
					promptSummary={questionAnswerPromptSummary}
					promptValue={promptsSettings.ragAnswerSystemPrompt}
					providerFieldId="question-answer-provider"
					providerLabel="问答模型"
					saveButtonLabel="保存文档问答配置"
					savingAiTaskConfig={savingAiTaskConfig}
					savingTaskConfig={savingQuestionAnswerConfig}
					selectedProvider={selectedQuestionAnswerProvider}
					summarizeLlmProviderProfile={summarizeLlmProviderProfile}
					title="文档问答"
					togglePromptExpanded={() => setQuestionAnswerPromptExpanded((current) => !current)}
					value={llmSettings.questionAnswerModelId ?? ""}
				/>
			</div>
		</section>
	);
}
