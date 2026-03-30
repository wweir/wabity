import { getSettingsPanelId, getSettingsTabId } from "../settingsShared";
import type { BindSectionBlockRef } from "../sectionViewShared";

export interface AboutSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
}

export function AboutSettingsSection({ bindSectionBlockRef }: AboutSettingsSectionProps) {
	return (
		<section
			aria-labelledby={getSettingsTabId("about")}
			className="settings-section"
			id={getSettingsPanelId("about")}
			role="tabpanel"
			tabIndex={0}
		>
			<div
				className="settings-editor-card settings-editor-card-subtle"
				id="about-overview"
				ref={bindSectionBlockRef("about-overview")}
			>
				<div className="settings-item">
					<span className="settings-info-label">版本</span>
					<span className="settings-info-value">0.1.0</span>
				</div>
				<div className="settings-item">
					<span className="settings-info-label">检查更新</span>
					<button className="settings-button" type="button">
						检查更新
					</button>
				</div>
			</div>
		</section>
	);
}
