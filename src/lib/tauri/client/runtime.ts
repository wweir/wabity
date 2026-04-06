import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ExecutionResult } from "../../../features/launcher/types";

export interface OcrTranslationResultEvent {
	sourceMode: "ocr" | "selection";
	sourceText: string;
	result: ExecutionResult;
}

export interface OcrTranslationStartedEvent {
	sourceMode: "ocr" | "selection";
	sourceText: string;
}

export interface OcrTranslationStreamEvent {
	sourceMode: "ocr" | "selection";
	sourceText: string;
	partialText: string;
}

export interface ExecutionProgressEvent {
	actionId: string;
	statusText: string;
	partialText?: string | null;
}

const lastLauncherWindowSizeByLabel = new Map<string, { width: number; height: number }>();
let currentWindowLabelPromise: Promise<string | null> | null = null;

function canUseTauriInvoke(): boolean {
	if (!isTauri()) {
		return false;
	}

	const tauriInternals = (globalThis as { __TAURI_INTERNALS__?: { invoke?: unknown } })
		.__TAURI_INTERNALS__;
	return typeof tauriInternals?.invoke === "function";
}

export function isDesktopRuntimeAvailable(): boolean {
	return canUseTauriInvoke();
}

export async function invokeDesktop<T>(
	command: string,
	args?: Record<string, unknown>,
): Promise<T> {
	if (args) {
		return invoke<T>(command, args);
	}

	return invoke<T>(command);
}

export async function invokeOrDefault<T>(
	command: string,
	fallbackValue: T | (() => T),
	args?: Record<string, unknown>,
): Promise<T> {
	if (!canUseTauriInvoke()) {
		return typeof fallbackValue === "function" ? (fallbackValue as () => T)() : fallbackValue;
	}

	return invokeDesktop<T>(command, args);
}

export async function invokeIfDesktop(
	command: string,
	args?: Record<string, unknown>,
): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	await invokeDesktop<void>(command, args);
}

export async function listenIfDesktop<T>(
	eventName: string,
	callback: (payload: T) => void,
): Promise<UnlistenFn | null> {
	if (!canUseTauriInvoke()) {
		return null;
	}

	return listen<T>(eventName, (event) => callback(event.payload));
}

export async function subscribeChannelIfDesktop<T>(
	command: string,
	unsubscribeCommand: string,
	callback: (payload: T) => void,
): Promise<UnlistenFn | null> {
	if (!canUseTauriInvoke()) {
		return null;
	}

	const channel = new Channel<T>();
	channel.onmessage = callback;
	await invokeDesktop(command, { onEvent: channel });
	return () => {
		channel.onmessage = () => {};
		void invokeIfDesktop(unsubscribeCommand, { channelId: channel.id });
	};
}

export async function resolveCurrentWindowLabel(): Promise<string | null> {
	if (!isDesktopRuntimeAvailable()) {
		return null;
	}

	if (!currentWindowLabelPromise) {
		currentWindowLabelPromise = Promise.resolve()
			.then(() => getCurrentWindow().label)
			.catch(() => null);
	}

	return currentWindowLabelPromise;
}

export async function resizeLauncherWindow(size: {
	width: number;
	height: number;
}): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	const nextWidth = Math.max(1, Math.ceil(size.width));
	const nextHeight = Math.max(1, Math.ceil(size.height));
	const windowLabel = (await resolveCurrentWindowLabel()) ?? "main";
	const lastLauncherWindowSize = lastLauncherWindowSizeByLabel.get(windowLabel) ?? null;

	if (
		lastLauncherWindowSize?.width === nextWidth &&
		lastLauncherWindowSize?.height === nextHeight
	) {
		return;
	}

	await invokeDesktop<void>("resize_launcher_window", {
		width: nextWidth,
		height: nextHeight,
		windowLabel,
	});

	lastLauncherWindowSizeByLabel.set(windowLabel, { width: nextWidth, height: nextHeight });
}
