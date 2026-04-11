import { useCallback, useEffect, useRef, useState } from "react";

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
import {
	matchActions,
	searchApps,
	searchFiles,
	searchProcesses,
} from "../../lib/tauri/client/launcher";

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
	const {
		currentFileNeedle,
		killSearchQuery,
		launcherMode,
		setError,
		suggestionMode,
		suggestionQuery,
		textBeforeCaret,
	} = args;
	const [actionMatches, setActionMatches] = useState<ActionMatch[]>([]);
	const [fileMatches, setFileMatches] = useState<FileSearchMatch[]>([]);
	const [appMatches, setAppMatches] = useState<InstalledAppMatch[]>([]);
	const [killMatches, setKillMatches] = useState<RunningProcessMatch[]>([]);
	const killSuggestionCacheRef = useRef<Map<string, RunningProcessMatch[]>>(new Map());
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
			setError(null);
		},
		[setError],
	);

	const warmKillSuggestionsFromCache = useCallback(
		(query: string) => {
			const normalizedQuery = query.trim().toLowerCase();
			if (!normalizedQuery) {
				return false;
			}

			const cache = killSuggestionCacheRef.current;
			const exact = cache.get(normalizedQuery);
			if (exact) {
				showSuggestions({ killMatches: exact });
				return true;
			}

			let bestPrefixMatches: RunningProcessMatch[] | null = null;
			let bestPrefixLength = 0;
			for (const [cachedQuery, cachedMatches] of cache.entries()) {
				if (cachedQuery.length <= bestPrefixLength || !normalizedQuery.startsWith(cachedQuery)) {
					continue;
				}
				bestPrefixLength = cachedQuery.length;
				bestPrefixMatches = cachedMatches;
			}

			if (!bestPrefixMatches) {
				return false;
			}

			const filteredMatches = bestPrefixMatches
				.filter((match) => {
					const pidText = `pid:${match.pid}`;
					return (
						match.displayName.toLowerCase().includes(normalizedQuery) ||
						match.processName.toLowerCase().includes(normalizedQuery) ||
						pidText.startsWith(normalizedQuery)
					);
				})
				.slice(0, 8);
			if (filteredMatches.length === 0) {
				return false;
			}

			showSuggestions({ killMatches: filteredMatches });
			return true;
		},
		[showSuggestions],
	);

	useEffect(() => {
		let cancelled = false;
		let debounceTimer: number | null = null;

		async function loadSuggestions() {
			const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
			if (suggestionMode === "file") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await searchFiles(currentFileNeedle, 8);
					console.debug("[launcher] file suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: currentFileNeedle.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({ fileMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "文件搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (suggestionMode === "action") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await matchActions(suggestionQuery);
					console.debug("[launcher] action suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: suggestionQuery.rawText.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({
							actionMatches: suggestionQuery.rawText.trim().startsWith("/")
								? nextMatches
								: nextMatches.slice(0, 8),
						});
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "动作匹配失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (suggestionMode === "app") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await searchApps(textBeforeCaret, 8);
					console.debug("[launcher] app suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: textBeforeCaret.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						showSuggestions({ appMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "应用搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (suggestionMode === "kill") {
				setSuggestionLoading(true);
				try {
					const nextMatches = await searchProcesses(killSearchQuery, 8);
					console.debug("[launcher] kill suggestions completed", {
						elapsedMs:
							(typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt,
						queryLength: killSearchQuery.trim().length,
						resultCount: nextMatches.length,
					});
					if (!cancelled) {
						killSuggestionCacheRef.current.set(
							killSearchQuery.trim().toLowerCase(),
							nextMatches,
						);
						showSuggestions({ killMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "进程搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setSuggestionLoading(false);
					}
				}

				return;
			}

			if (!launcherMode) {
				resetSuggestions();
				setSuggestionLoading(false);
				return;
			}

			resetSuggestions();
			setSuggestionLoading(false);
		}

		if (suggestionMode === "file") {
			if (!isFileSearchReady(currentFileNeedle)) {
				resetSuggestions(true);
				setSuggestionLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, fileSearchDebounceMs);
		} else if (suggestionMode === "app") {
			if (!isAppSearchReady(textBeforeCaret)) {
				resetSuggestions(true);
				setSuggestionLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, appSearchDebounceMs);
		} else if (suggestionMode === "kill") {
			if (!isKillSearchReady(killSearchQuery)) {
				resetSuggestions(true);
				setSuggestionLoading(false);
				return () => {
					cancelled = true;
				};
			}

			warmKillSuggestionsFromCache(killSearchQuery);
			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, killSearchDebounceMs);
		} else if (suggestionMode === "action") {
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
		currentFileNeedle,
		killSearchQuery,
		launcherMode,
		setError,
		suggestionMode,
		suggestionQuery,
		textBeforeCaret,
		resetSuggestions,
		showSuggestions,
		warmKillSuggestionsFromCache,
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
