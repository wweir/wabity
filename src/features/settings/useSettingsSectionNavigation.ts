import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";

import { settingsQuickLinks, settingsSections } from "./settingsShared";
import type { SettingsSectionId } from "./settingsTypes";

interface UseSettingsSectionNavigationArgs {
	activeSection: SettingsSectionId;
	setActiveSection: (sectionId: SettingsSectionId) => void;
	setSelectedSkillId: (skillId: string) => void;
}

export function useSettingsSectionNavigation({
	activeSection,
	setActiveSection,
	setSelectedSkillId,
}: UseSettingsSectionNavigationArgs) {
	const sectionTabRefs = useRef<Record<SettingsSectionId, HTMLButtonElement | null>>({
		general: null,
		prompts: null,
		llm: null,
		rag: null,
		acp: null,
		mcp: null,
		skills: null,
		about: null,
	});
	const sectionBlockRefs = useRef<Record<string, HTMLElement | null>>({});
	const contentRef = useRef<HTMLDivElement | null>(null);
	const activeQuickLinks = settingsQuickLinks[activeSection];
	const [activeSectionBlockId, setActiveSectionBlockId] = useState<string | null>(
		settingsQuickLinks.general[0]?.id ?? null,
	);

	function handleSectionTabKeyDown(
		event: ReactKeyboardEvent<HTMLButtonElement>,
		sectionId: SettingsSectionId,
	) {
		const currentIndex = settingsSections.findIndex((section) => section.id === sectionId);
		if (currentIndex < 0) {
			return;
		}

		let nextIndex: number | null = null;
		switch (event.key) {
			case "ArrowRight":
			case "ArrowDown":
				nextIndex = (currentIndex + 1) % settingsSections.length;
				break;
			case "ArrowLeft":
			case "ArrowUp":
				nextIndex = (currentIndex - 1 + settingsSections.length) % settingsSections.length;
				break;
			case "Home":
				nextIndex = 0;
				break;
			case "End":
				nextIndex = settingsSections.length - 1;
				break;
			default:
				return;
		}

		event.preventDefault();
		const nextSectionId = settingsSections[nextIndex]?.id;
		if (!nextSectionId) {
			return;
		}

		setActiveSection(nextSectionId);
		sectionTabRefs.current[nextSectionId]?.focus();
	}

	function bindSectionBlockRef(blockId: string) {
		return (element: HTMLElement | null) => {
			sectionBlockRefs.current[blockId] = element;
		};
	}

	function scrollToSectionBlock(blockId: string) {
		const target = sectionBlockRefs.current[blockId];
		if (!target) {
			return;
		}

		const prefersReducedMotion =
			typeof window !== "undefined" &&
			typeof window.matchMedia === "function" &&
			window.matchMedia("(prefers-reduced-motion: reduce)").matches;

		setActiveSectionBlockId(blockId);
		target.scrollIntoView({
			behavior: prefersReducedMotion ? "auto" : "smooth",
			block: "start",
			inline: "nearest",
		});
	}

	function handleViewSkill(skillId: string) {
		setSelectedSkillId(skillId);
		scrollToSectionBlock("skills-detail");
	}

	function handleSelectSection(sectionId: SettingsSectionId) {
		setActiveSection(sectionId);
		setActiveSectionBlockId(settingsQuickLinks[sectionId][0]?.id ?? null);

		const root = contentRef.current;
		if (!root) {
			return;
		}

		root.scrollTo({ top: 0 });
	}

	useEffect(() => {
		setActiveSectionBlockId(settingsQuickLinks[activeSection][0]?.id ?? null);
	}, [activeSection]);

	useEffect(() => {
		const root = contentRef.current;
		if (!root) {
			return;
		}

		const observedBlocks = activeQuickLinks
			.map((link) => sectionBlockRefs.current[link.id])
			.filter((element): element is HTMLElement => element instanceof HTMLElement);

		if (observedBlocks.length === 0) {
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				const candidate = entries
					.filter((entry) => entry.isIntersecting)
					.sort(
						(left, right) =>
							right.intersectionRatio - left.intersectionRatio ||
							left.boundingClientRect.top - right.boundingClientRect.top,
					)[0];

				if (!(candidate?.target instanceof HTMLElement)) {
					return;
				}

				const nextBlockId = candidate.target.id;
				setActiveSectionBlockId((current) => (current === nextBlockId ? current : nextBlockId));
			},
			{
				root,
				rootMargin: "-12% 0px -58% 0px",
				threshold: [0.2, 0.4, 0.65],
			},
		);

		observedBlocks.forEach((block) => observer.observe(block));
		return () => {
			observer.disconnect();
		};
	}, [activeQuickLinks]);

	return {
		activeQuickLinks,
		activeSectionBlockId,
		bindSectionBlockRef,
		contentRef,
		handleSectionTabKeyDown,
		handleSelectSection,
		handleViewSkill,
		scrollToSectionBlock,
		sectionTabRefs,
	};
}
