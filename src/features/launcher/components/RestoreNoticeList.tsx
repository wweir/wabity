import type { AcpRestoreNotice } from "../../../lib/tauri/types";

interface RestoreNoticeListProps {
	restoreNotices: AcpRestoreNotice[];
}

export function RestoreNoticeList({ restoreNotices }: RestoreNoticeListProps) {
	if (restoreNotices.length === 0) {
		return null;
	}

	return (
		<section className="restore-notice-list" aria-label="会话恢复提示">
			{restoreNotices.map((notice) => (
				<article
					className="restore-notice-item"
					key={`${notice.sessionId}-${notice.workspaceRoot}`}
				>
					<strong className="restore-notice-title">恢复失败</strong>
					<span className="restore-notice-message">{notice.message}</span>
				</article>
			))}
		</section>
	);
}
