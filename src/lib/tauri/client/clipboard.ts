import {
	browserClipboardHistorySnapshot,
} from "./defaults";
import { invokeIfDesktop, invokeOrDefault, listenIfDesktop } from "./runtime";
import type {
	ClipboardHistorySnapshot,
	InsertClipboardHistoryTextIntoLauncherEvent,
	OpenClipboardHistoryPanelEvent,
	RevealLauncherMainPanelEvent,
} from "../types";

export async function dismissClipboardHistoryPanel(): Promise<void> {
	return invokeIfDesktop("dismiss_clipboard_history_panel");
}

export async function onOpenClipboardHistoryPanel(
	callback: (payload: OpenClipboardHistoryPanelEvent) => void,
) {
	return listenIfDesktop("open-clipboard-history-panel", callback);
}

export async function onRevealLauncherMainPanel(
	callback: (payload: RevealLauncherMainPanelEvent) => void,
) {
	return listenIfDesktop("reveal-launcher-main-panel", callback);
}

export async function onInsertClipboardHistoryTextIntoLauncher(
	callback: (payload: InsertClipboardHistoryTextIntoLauncherEvent) => void,
) {
	return listenIfDesktop("insert-clipboard-history-text-into-launcher", callback);
}

export async function insertClipboardHistoryTextIntoLauncher(text: string): Promise<void> {
	return invokeIfDesktop("insert_clipboard_history_text_into_launcher", { text });
}

export async function getClipboardHistory(): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("get_clipboard_history", browserClipboardHistorySnapshot);
}

export async function toggleClipboardHistoryEntryPin(
	entryId: string,
): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("toggle_clipboard_history_entry_pin", browserClipboardHistorySnapshot, {
		entryId,
	});
}

export async function deleteClipboardHistoryEntry(
	entryId: string,
): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("delete_clipboard_history_entry", browserClipboardHistorySnapshot, {
		entryId,
	});
}

export async function pasteClipboardHistoryEntry(
	entryId: string,
): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("paste_clipboard_history_entry", browserClipboardHistorySnapshot, {
		entryId,
	});
}

export async function onClipboardHistoryUpdated(
	callback: (snapshot: ClipboardHistorySnapshot) => void,
) {
	return listenIfDesktop("clipboard-history-updated", callback);
}
