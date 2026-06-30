import type {
	AppearanceSettings,
	GeneralSettings,
	NotificationSettings,
	ShortcutConfig,
} from "../../../lib/tauri/types";
import { ShortcutRecorderField, getSettingsPanelId, getSettingsTabId } from "../settingsShared";
import type { BindSectionBlockRef } from "../sectionViewShared";

export interface GeneralSettingsSectionProps {
	generalSettings: GeneralSettings;
	notificationSettings: NotificationSettings;
	appearanceSettings: AppearanceSettings;
	shortcutSettings: ShortcutConfig;
	savingSettings: boolean;
	savingShortcutKey: keyof ShortcutConfig | null;
	bindSectionBlockRef: BindSectionBlockRef;
	isRecording: (key: keyof ShortcutConfig) => boolean;
	onShortcutClick: (key: keyof ShortcutConfig) => void;
	onSaveAppSettings: (
		nextGeneral: GeneralSettings,
		nextNotification: NotificationSettings,
		nextAppearance: AppearanceSettings,
	) => Promise<void>;
	onSaveNotificationSettings: (nextNotification: NotificationSettings) => Promise<void>;
}

export function GeneralSettingsSection({
	generalSettings,
	notificationSettings,
	appearanceSettings,
	shortcutSettings,
	savingSettings,
	savingShortcutKey,
	bindSectionBlockRef,
	isRecording,
	onShortcutClick,
	onSaveAppSettings,
	onSaveNotificationSettings,
}: GeneralSettingsSectionProps) {
	return (
		<section
			aria-labelledby={getSettingsTabId("general")}
			className="settings-section settings-section-general"
			id={getSettingsPanelId("general")}
			role="tabpanel"
			tabIndex={0}
		>
			<div className="settings-general-basic-list">
				<div className="settings-item">
					<label className="settings-label">
						<span>开机自启动</span>
						<input
							checked={generalSettings.autoStart}
							className="settings-toggle"
							onChange={(event) =>
								void onSaveAppSettings(
									{ ...generalSettings, autoStart: event.target.checked },
									notificationSettings,
									appearanceSettings,
								)
							}
							disabled={savingSettings}
							type="checkbox"
						/>
					</label>
				</div>
				<div className="settings-item">
					<label className="settings-label">
						<span>在 Dock 中显示</span>
						<input
							checked={generalSettings.showInDock}
							className="settings-toggle"
							onChange={(event) =>
								void onSaveAppSettings(
									{ ...generalSettings, showInDock: event.target.checked },
									notificationSettings,
									appearanceSettings,
								)
							}
							disabled={savingSettings}
							type="checkbox"
						/>
					</label>
				</div>
				<div className="settings-item">
					<label className="settings-label">
						<span>界面语言</span>
						<select
							className="settings-select"
							value={generalSettings.language}
							onChange={(event) =>
								void onSaveAppSettings(
									{ ...generalSettings, language: event.target.value },
									notificationSettings,
									appearanceSettings,
								)
							}
							disabled={savingSettings}
						>
							<option value="zh-CN">简体中文</option>
							<option value="en-US">English</option>
						</select>
					</label>
				</div>
			</div>

			<div
				className="settings-editor-card settings-editor-card-subtle settings-editor-card-shortcuts settings-general-card"
				id="general-shortcuts"
				ref={bindSectionBlockRef("general-shortcuts")}
			>
				<div className="settings-editor-card-header settings-general-card-header">
					<div className="settings-general-card-copy">
						<h3 className="settings-subsection-title">快捷键</h3>
						<span
							className="settings-help-text settings-help-text-tight"
							id="general-shortcuts-instructions"
						>
							快捷键仍然单独保存。按 Enter 或空格开始录制，按 Escape 取消。
						</span>
					</div>
				</div>
				<ShortcutRecorderField
					instructionsId="general-shortcuts-instructions"
					isRecording={isRecording("toggle_launcher")}
					isSaving={savingShortcutKey === "toggle_launcher"}
					label="打开启动器"
					onActivate={() => onShortcutClick("toggle_launcher")}
					shortcutValue={shortcutSettings.toggle_launcher}
					statusId="shortcut-toggle-launcher-status"
					triggerId="shortcut-toggle-launcher-trigger"
				/>
				<ShortcutRecorderField
					instructionsId="general-shortcuts-instructions"
					isRecording={isRecording("ocr_translate")}
					isSaving={savingShortcutKey === "ocr_translate"}
					label="翻译选中文本，未选中时 OCR"
					onActivate={() => onShortcutClick("ocr_translate")}
					shortcutValue={shortcutSettings.ocr_translate}
					statusId="shortcut-ocr-translate-status"
					triggerId="shortcut-ocr-translate-trigger"
				/>
				<ShortcutRecorderField
					instructionsId="general-shortcuts-instructions"
					isRecording={isRecording("open_clipboard_history")}
					isSaving={savingShortcutKey === "open_clipboard_history"}
					label="打开历史剪贴板"
					onActivate={() => onShortcutClick("open_clipboard_history")}
					shortcutValue={shortcutSettings.open_clipboard_history}
					statusId="shortcut-open-clipboard-history-status"
					triggerId="shortcut-open-clipboard-history-trigger"
				/>
			</div>

			<div
				className="settings-editor-card settings-editor-card-subtle settings-general-card"
				id="general-notifications"
				ref={bindSectionBlockRef("general-notifications")}
			>
				<div className="settings-editor-card-header settings-general-card-header">
					<div className="settings-general-card-copy">
						<h3 className="settings-subsection-title">通知</h3>
						<span className="settings-help-text settings-help-text-tight">
							控制完成通知的触发范围和摘要粒度。通知样式继续使用系统原生外观。
						</span>
					</div>
				</div>
				<div className="settings-general-field-list">
					<div className="settings-item">
						<label className="settings-label">
							<span>启用完成通知</span>
							<input
								checked={notificationSettings.enabled}
								className="settings-toggle"
								onChange={(event) =>
									void onSaveNotificationSettings({
										...notificationSettings,
										enabled: event.target.checked,
									})
								}
								disabled={savingSettings}
								type="checkbox"
							/>
						</label>
					</div>
					<div className="settings-item">
						<label className="settings-label">
							<span>通知内容</span>
							<select
								className="settings-select"
								value={notificationSettings.contentPreview}
								onChange={(event) =>
									void onSaveNotificationSettings({
										...notificationSettings,
										contentPreview: event.target.value as NotificationSettings["contentPreview"],
									})
								}
								disabled={savingSettings || !notificationSettings.enabled}
							>
								<option value="brief">显示简短响应摘要</option>
								<option value="hidden">只显示完成状态</option>
							</select>
						</label>
					</div>
					<div className="settings-item">
						<label className="settings-label">
							<span>文档问答完成后通知</span>
							<input
								checked={notificationSettings.notifyQuestionAnswerCompletion}
								className="settings-toggle"
								onChange={(event) =>
									void onSaveNotificationSettings({
										...notificationSettings,
										notifyQuestionAnswerCompletion: event.target.checked,
									})
								}
								disabled={savingSettings || !notificationSettings.enabled}
								type="checkbox"
							/>
						</label>
					</div>
					<div className="settings-item">
						<label className="settings-label">
							<span>Agent 完成后通知</span>
							<input
								checked={notificationSettings.notifyAcpPromptCompletion}
								className="settings-toggle"
								onChange={(event) =>
									void onSaveNotificationSettings({
										...notificationSettings,
										notifyAcpPromptCompletion: event.target.checked,
									})
								}
								disabled={savingSettings || !notificationSettings.enabled}
								type="checkbox"
							/>
						</label>
					</div>
					<div className="settings-item">
						<label className="settings-label">
							<span>仅在 Launcher 位于后台时通知</span>
							<input
								checked={notificationSettings.onlyWhenLauncherInBackground}
								className="settings-toggle"
								onChange={(event) =>
									void onSaveNotificationSettings({
										...notificationSettings,
										onlyWhenLauncherInBackground: event.target.checked,
									})
								}
								disabled={savingSettings || !notificationSettings.enabled}
								type="checkbox"
							/>
						</label>
					</div>
				</div>
			</div>

			<div
				className="settings-editor-card settings-editor-card-subtle settings-general-card"
				id="general-appearance"
				ref={bindSectionBlockRef("general-appearance")}
			>
				<div className="settings-editor-card-header settings-general-card-header">
					<div className="settings-general-card-copy">
						<h3 className="settings-subsection-title">外观</h3>
						<span className="settings-help-text settings-help-text-tight">
							主题和字号会立即应用到当前界面。
						</span>
					</div>
				</div>
				<div className="settings-general-field-list">
					<div className="settings-item">
						<label className="settings-label">
							<span>主题</span>
							<select
								className="settings-select"
								value={appearanceSettings.theme}
								onChange={(event) =>
									void onSaveAppSettings(generalSettings, notificationSettings, {
										...appearanceSettings,
										theme: event.target.value,
									})
								}
								disabled={savingSettings}
							>
								<option value="auto">跟随系统</option>
								<option value="light">浅色</option>
								<option value="dark">深色</option>
							</select>
						</label>
					</div>
					<div className="settings-item">
						<label className="settings-label">
							<span>字体大小</span>
							<select
								className="settings-select"
								value={appearanceSettings.fontSize}
								onChange={(event) =>
									void onSaveAppSettings(generalSettings, notificationSettings, {
										...appearanceSettings,
										fontSize: event.target.value,
									})
								}
								disabled={savingSettings}
							>
								<option value="small">小</option>
								<option value="medium">中</option>
								<option value="large">大</option>
							</select>
						</label>
					</div>
				</div>
			</div>
		</section>
	);
}
