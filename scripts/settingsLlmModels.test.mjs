import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { hydrateAppSettings } from "../src/lib/tauri/client/settingsNormalization.ts";

const settingsPagePath = new URL("../src/features/settings/SettingsPage.tsx", import.meta.url);
const settingsStatePath = new URL("../src/features/settings/settingsState.ts", import.meta.url);
const llmSectionPath = new URL(
	"../src/features/settings/sections/LlmSettingsSection.tsx",
	import.meta.url,
);

async function readSource(url) {
	return readFile(url, "utf8");
}

test("selected model kind updates target the currently selected model id", async () => {
	const source = await readSource(settingsPagePath);

	assert.match(
		source,
		/function handleLlmProviderKindChange[\s\S]*?applyLlmProviderKind\(\s*provider,\s*kind,\s*getSelectedProviderModel\(provider\)\.id,\s*\)/,
	);
});

test("template and managed base url changes invalidate cached model catalogs", async () => {
	const source = await readSource(settingsPagePath);

	assert.match(
		source,
		/function handleLlmProviderTemplateChange[\s\S]*?clearLlmModelCatalog\(providerId\)/,
	);
	assert.match(
		source,
		/function handleBuiltinProviderManagedBaseUrlChange[\s\S]*?clearLlmModelCatalog\(providerId\)/,
	);
});

test("template and connection mutations clear identity hints for every model in the provider group", async () => {
	const settingsPageSource = await readSource(settingsPagePath);
	const settingsStateSource = await readSource(settingsStatePath);

	assert.match(
		settingsPageSource,
		/function handleLlmProviderTemplateChange[\s\S]*?clearProviderModelIdentityHints\(/,
	);
	assert.match(
		settingsPageSource,
		/function handleBuiltinProviderManagedBaseUrlChange[\s\S]*?clearProviderModelIdentityHints\(/,
	);
	assert.match(
		settingsStateSource,
		/export function clearProviderModelIdentityHints\(provider: LlmProviderConfig\)[\s\S]*?provider\.models\.map/,
	);
});

test("hydrateAppSettings backfills provider modelConfig from the first model when desktop payload omits it", () => {
	const hydrated = hydrateAppSettings({
		general: {
			autoStart: false,
			showInDock: true,
			language: "zh-CN",
		},
		notification: {
			enabled: false,
			notifyQuestionAnswerCompletion: true,
			notifyAcpPromptCompletion: true,
			onlyWhenLauncherInBackground: true,
			contentPreview: "brief",
		},
		appearance: {
			theme: "auto",
			fontSize: "medium",
		},
		prompts: {
			translationPrompt: "",
			ragAnswerSystemPrompt: "",
		},
		llm: {
			providers: [
				{
					id: "provider-1",
					name: "OpenAI",
					baseUrl: "https://api.openai.com/v1",
					apiKey: "",
					protocol: "responses",
					models: [
						{
							id: "model-1",
							modelType: "llm",
							model: "gpt-5.4-mini",
							modelIdentityHint: null,
							builtinPresetModelId: null,
							supportsMultimodal: true,
							supportsStateful: true,
						},
					],
					builtinPresetId: null,
					managedBaseUrl: false,
				},
			],
			translationModelId: null,
			questionAnswerModelId: null,
		},
		ocr: {
			provider: "system",
			llmModelId: null,
		},
		rag: {
			sourceDirectories: [],
			ignoreGlobs: [],
			embeddingModelId: null,
		},
	});

	assert.equal(hydrated.llm.providers[0]?.modelConfig.id, "model-1");
	assert.equal(hydrated.llm.providers[0]?.modelConfig.model, "gpt-5.4-mini");
});

test("llm settings section computes capability state from the selected model id instead of provider default order", async () => {
	const source = await readSource(llmSectionPath);

	assert.match(source, /providerCanHandleOcr\(selectedLlmProvider,\s*selectedProviderModelId\)/);
	assert.match(source, /providerIsLlmModel\(selectedLlmProvider,\s*selectedProviderModelId\)/);
	assert.match(
		source,
		/providerHasResponsesModel\(\s*selectedLlmProvider,\s*selectedProviderModelId/s,
	);
});
