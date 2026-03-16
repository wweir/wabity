import type { WorkspaceState } from "../../lib/tauri/types";

export interface WorkspaceBreadcrumb {
	label: string;
	path: string;
}

export function formatWorkspacePath(path: string, workspace: WorkspaceState) {
	if (!path) {
		return "";
	}

	const normalizedPath = path.replace(/\\/g, "/");
	const normalizedHomePath = workspace.homePath?.replace(/\\/g, "/") ?? null;

	if (
		workspace.displayHomeAsTilde &&
		normalizedHomePath &&
		(normalizedPath === normalizedHomePath || normalizedPath.startsWith(`${normalizedHomePath}/`))
	) {
		const suffix = normalizedPath.slice(normalizedHomePath.length);
		return suffix ? `~${suffix}` : "~";
	}

	return path;
}

export function buildWorkspaceBreadcrumbs(workspace: WorkspaceState): WorkspaceBreadcrumb[] {
	const { rootPath } = workspace;
	if (!rootPath) {
		return [];
	}

	const normalizedHomePath = workspace.homePath?.replace(/\\/g, "/") ?? null;
	const normalized = rootPath.replace(/\\/g, "/");
	if (
		workspace.displayHomeAsTilde &&
		normalizedHomePath &&
		(normalized === normalizedHomePath || normalized.startsWith(`${normalizedHomePath}/`))
	) {
		const relative = normalized.slice(normalizedHomePath.length).replace(/^\/+/, "");
		const parts = relative ? relative.split("/").filter((part) => part.length > 0) : [];
		const breadcrumbs = [{ label: "~", path: workspace.homePath ?? rootPath }];

		parts.forEach((part, index) => {
			const path =
				normalizedHomePath + (index === 0 ? `/${part}` : `/${parts.slice(0, index + 1).join("/")}`);
			breadcrumbs.push({ label: part, path });
		});

		return breadcrumbs;
	}

	const isAbsolute = normalized.startsWith("/");
	const parts = normalized.split("/").filter((part) => part.length > 0);
	const breadcrumbs: WorkspaceBreadcrumb[] = [];

	if (isAbsolute) {
		breadcrumbs.push({ label: "/", path: "/" });
	}

	parts.forEach((part, index) => {
		const prefix = isAbsolute ? "/" : "";
		const path = `${prefix}${parts.slice(0, index + 1).join("/")}`;
		breadcrumbs.push({ label: part, path });
	});

	return breadcrumbs;
}
