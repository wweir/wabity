import { useEffect, useState } from "react";

import { getRagRuntimeStatus, onRagRuntimeStatus } from "../../lib/tauri/client";
import type { RagRuntimeStatus } from "../../lib/tauri/types";

const initialRagRuntimeStatus: RagRuntimeStatus = {
	phase: "idle",
	scannedFileCount: 0,
	completedFileCount: 0,
	totalFileCount: 0,
	pendingFileCount: 0,
	warningCount: 0,
	recentWarnings: [],
	lastError: null,
	updatedAtMs: 0,
};

export function useRagRuntimeStatus(): RagRuntimeStatus {
	const [ragRuntimeStatus, setRagRuntimeStatus] =
		useState<RagRuntimeStatus>(initialRagRuntimeStatus);

	useEffect(() => {
		let active = true;
		let unlisten: (() => void) | null = null;

		void getRagRuntimeStatus()
			.then((nextStatus) => {
				if (active) {
					setRagRuntimeStatus(nextStatus);
				}
			})
			.catch(() => {
				if (active) {
					setRagRuntimeStatus((current) => current);
				}
			});
		void onRagRuntimeStatus((nextStatus) => {
			if (active) {
				setRagRuntimeStatus(nextStatus);
			}
		})
			.then((dispose) => {
				if (!dispose) {
					return;
				}

				if (!active) {
					dispose();
					return;
				}

				unlisten = dispose;
			})
			.catch((error: unknown) => {
				console.warn("failed to subscribe RAG runtime status", error);
			});

		return () => {
			active = false;
			unlisten?.();
		};
	}, []);

	return ragRuntimeStatus;
}
