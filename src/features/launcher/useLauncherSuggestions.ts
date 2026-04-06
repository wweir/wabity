import { useCallback, useEffect, useState } from "react";

import {
	appSearchDebounceMs,
	fileSearchDebounceMs,
	getErrorMessage,
	isAppSearchReady,
	isFileSearchReady,
	isKillSearchReady,
	killSearchDebounceMs,
	type SuggestionMode,
} from "./query";
import type {
	ActionMatch,
	FileSearchMatch,
	InstalledAppMatch,
	QueryPayload,
	RunningProcessMatch,
} from "./types";
import { matchActions, searchApps, searchFiles, searchProcesses } from "../../lib/tauri/client";

interface UseLauncherSuggestionsArgs {
	currentFileNeedle: string;
	killSearchQuery: string;
	launcherMode: boolean;
	setError: (message: string | null) => void;
	suggestionMode: SuggestionMode;
	suggestionQuery: QueryPayload;
	textBeforeCaret: string;
}

export function useLauncherSuggestions(args: UseLauncherSuggestionsArgs) {
	const [actionMatches, setActionMatches] = useState<ActionMatch[]>([]);
	const [fileMatches, setFileMatches] = useState<FileSearchMatch[]>([]);
	const [appMatches, setAppMatches] = useState<InstalledAppMatch[]>([]);
	const [killMatches, setKillMatches] = useState<RunningProcessMatch[]>([]);
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [suggestionLoading, setSuggestionLoading] = useState(false);
	const [suggestionsHidden, setSuggestionsHidden] = useState(false);

	const resetSuggestions = useCallback((resetSelectedIndex: boolean = false) => {
		setActionMatches([]);
		setFileMatches([]);
		setAppMatches([]);
		setKillMatches([]);
		if (resetSelectedIndex) {
			setSelectedIndex(0);
		}
	}, []);

	const showSuggestions = useCallback(
		(nextSuggestions: {
			actionMatches?: ActionMatch[];
			fileMatches?: FileSearchMatch[];
			appMatches?: InstalledAppMatch[];
			killMatches?: RunningProcessMatch[];
		}) => {
			setActionMatches(nextSuggestions.actionMatches ?? []);
			setFileMatches(nextSuggestions.fileMatches ?? []);
			setAppMatches(nextSuggestions.appMatches ?? []);
			setKillMatches(nextSuggestions.killMatches ?? []);
			setSuggestionsHidden(false);
			setSelectedIndex(0);
			args.setError(null);
		},
		[args.setError],
	);

	useEffect(() => {
		let cancelled = false;
		let debounceTimer: number | null = null;

		async function loadSuggestions() {
			const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
			if (args.suggestionMode === "file") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await searchFiles(args.currentFileNeedle, 8);
					console.debug("[launcher] file suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: args.currentFileNeedle.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({ fileMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						args.setError(getErrorMessage(loadError, "文件搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (args.suggestionMode === "action") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await matchActions(args.suggestionQuery);
					console.debug("[launcher] action suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: args.suggestionQuery.rawText.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({
							actionMatches: args.suggestionQuery.rawText.trim().startsWith("/")
								? nextMatches
								: nextMatches.slice(0, 8),
						});
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						args.setError(getErrorMessage(loadError, "动作匹配失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (args.suggestionMode === "app") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await searchApps(args.textBeforeCaret, 8);
					console.debug("[launcher] app suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: args.textBeforeCaret.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({ appMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						args.setError(getErrorMessage(loadError, "应用搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (args.suggestionMode === "kill") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await searchProcesses(args.killSearchQuery, 8);
					console.debug("[launcher] kill suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: args.killSearchQuery.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({ killMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						args.setError(getErrorMessage(loadError, "进程搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (!args.launcherMode) {
				resetSuggestions();
				setSuggestionLoading(false);
				return;
			}

			resetSuggestions();
			setSuggestionLoading(false);
		}

		if (args.suggestionMode === "file") {
			if (!isFileSearchReady(args.currentFileNeedle)) {
				resetSuggestions(true);
				setSuggestionLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, fileSearchDebounceMs);
		} else if (args.suggestionMode === "app") {
			if (!isAppSearchReady(args.textBeforeCaret)) {
				resetSuggestions(true);
				setSuggestionLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, appSearchDebounceMs);
		} else if (args.suggestionMode === "kill") {
			if (!isKillSearchReady(args.killSearchQuery)) {
				resetSuggestions(true);
				setSuggestionLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, killSearchDebounceMs);
		} else if (args.suggestionMode === "action") {
			void loadSuggestions();
		} else {
			resetSuggestions();
			setSuggestionLoading(false);
		}

		return () => {
			cancelled = true;
			if (debounceTimer !== null) {
				window.clearTimeout(debounceTimer);
			}
		};
	}, [
		args.currentFileNeedle,
		args.killSearchQuery,
		args.launcherMode,
		args.setError,
		args.suggestionMode,
		args.suggestionQuery,
		args.textBeforeCaret,
		resetSuggestions,
		showSuggestions,
	]);

	return {
		actionMatches,
		appMatches,
		fileMatches,
		killMatches,
		resetSuggestions,
		selectedIndex,
		setSelectedIndex,
		showSuggestions,
		suggestionLoading,
		suggestionsHidden,
		setSuggestionsHidden,
	};
}
