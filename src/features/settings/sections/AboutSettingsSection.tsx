import { useEffect, useRef, useState } from "react";
import { getVersion, getName } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { getSettingsPanelId, getSettingsTabId } from "../settingsShared";
import type { BindSectionBlockRef } from "../sectionViewShared";
import {
	RELEASE_CHECK_TIMEOUT_MS,
	resolveReleaseUpdate,
	type GithubReleaseSummary,
} from "./aboutUpdate";

const GITHUB_REPO = "wweir/wabity";
const GITHUB_URL = `https://github.com/${GITHUB_REPO}`;
const RELEASES_API = `https://api.github.com/repos/${GITHUB_REPO}/releases/latest`;

type UpdateStatus = "idle" | "checking" | "up-to-date" | "available" | "error";

export interface AboutSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	onOpenUrl: (url: string) => void;
}

export function AboutSettingsSection({
	bindSectionBlockRef,
	onOpenUrl,
}: AboutSettingsSectionProps) {
	const [appName, setAppName] = useState("Wabity");
	const [appVersion, setAppVersion] = useState("");
	const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
	const [latestVersion, setLatestVersion] = useState("");
	const [releaseUrl, setReleaseUrl] = useState("");
	const updateCheckAbortRef = useRef<AbortController | null>(null);

	useEffect(() => {
		if (!isTauri()) {
			return;
		}

		void getName().then(setAppName);
		void getVersion().then(setAppVersion);

		return () => {
			updateCheckAbortRef.current?.abort();
			updateCheckAbortRef.current = null;
		};
	}, []);

	async function checkForUpdates() {
		if (!appVersion) {
			return;
		}

		updateCheckAbortRef.current?.abort();
		const controller = new AbortController();
		updateCheckAbortRef.current = controller;
		let timedOut = false;
		const timeoutId = window.setTimeout(() => {
			timedOut = true;
			controller.abort();
		}, RELEASE_CHECK_TIMEOUT_MS);

		setLatestVersion("");
		setReleaseUrl("");
		setUpdateStatus("checking");

		try {
			const response = await fetch(RELEASES_API, {
				headers: {
					Accept: "application/vnd.github+json",
				},
				signal: controller.signal,
			});
			if (!response.ok) {
				throw new Error(`GitHub API ${response.status}`);
			}

			const release = (await response.json()) as GithubReleaseSummary;
			const nextUpdate = resolveReleaseUpdate(appVersion, release, `${GITHUB_URL}/releases/latest`);
			setLatestVersion(nextUpdate.latestVersion);
			setReleaseUrl(nextUpdate.releaseUrl);
			setUpdateStatus(nextUpdate.status);
		} catch {
			if (controller.signal.aborted && !timedOut) {
				return;
			}

			setUpdateStatus("error");
		} finally {
			window.clearTimeout(timeoutId);
			if (updateCheckAbortRef.current === controller) {
				updateCheckAbortRef.current = null;
			}
		}
	}

	const updateLabel =
		updateStatus === "checking"
			? "正在检查…"
			: updateStatus === "up-to-date"
				? "已是最新"
				: updateStatus === "available"
					? `有新版本 ${latestVersion}`
					: updateStatus === "error"
						? "检查失败"
						: "";

	return (
		<section
			aria-labelledby={getSettingsTabId("about")}
			className="settings-section"
			id={getSettingsPanelId("about")}
			role="tabpanel"
			tabIndex={0}
		>
			<div
				className="settings-editor-card settings-editor-card-subtle settings-about-card"
				id="about-overview"
				ref={bindSectionBlockRef("about-overview")}
			>
				<div className="settings-editor-card-header">
					<div className="settings-acp-sidebar-copy">
						<strong className="settings-subsection-title">{appName}</strong>
						<span className="settings-help-text settings-help-text-tight">
							本地优先的翻译、OCR 与文档问答工具，通过 LLM 和 RAG 驱动日常文本工作流。
						</span>
					</div>
				</div>
				<div className="settings-item">
					<span className="settings-info-label">版本</span>
					<span className="settings-info-value">
						{appVersion || "—"}
						{updateLabel ? ` · ${updateLabel}` : null}
					</span>
				</div>
				<div className="settings-item">
					<span className="settings-info-label">更新</span>
					{updateStatus === "available" ? (
						<button
							className="settings-text-link"
							onClick={() => onOpenUrl(releaseUrl)}
							type="button"
						>
							前往下载
						</button>
					) : (
						<button
							className="settings-button settings-button-compact"
							disabled={updateStatus === "checking" || !appVersion}
							onClick={checkForUpdates}
							type="button"
						>
							{updateStatus === "checking" ? "检查中…" : "检查更新"}
						</button>
					)}
				</div>
				<div className="settings-item">
					<span className="settings-info-label">源码</span>
					<button
						className="settings-text-link"
						onClick={() => onOpenUrl(GITHUB_URL)}
						type="button"
					>
						GitHub
					</button>
				</div>
			</div>
		</section>
	);
}
