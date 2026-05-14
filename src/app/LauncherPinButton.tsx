interface LauncherPinButtonProps {
	className?: string;
	pinned: boolean;
	onToggle: () => void;
}

export function LauncherPinButton({ className, pinned, onToggle }: LauncherPinButtonProps) {
	const label = pinned ? "取消固定窗口" : "固定窗口";

	return (
		<button
			aria-label={label}
			aria-pressed={pinned}
			className={["launcher-pin-button", pinned ? "launcher-pin-button-active" : null, className]
				.filter(Boolean)
				.join(" ")}
			onClick={onToggle}
			title={label}
			type="button"
		>
			<svg aria-hidden="true" className="launcher-pin-button-icon" viewBox="0 0 24 24">
				<path d="M14.5 3.5 20.5 9.5" />
				<path d="M8.8 13.2 3.8 18.2" />
				<path d="M7.2 8.4 15.6 16.8" />
				<path d="M6.2 9.4 10.4 5.2 18.8 13.6 14.6 17.8z" />
			</svg>
		</button>
	);
}
