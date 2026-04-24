import type { AppSettings, LlmModelConfig, LlmProviderConfig, LlmSettings } from "../types";

function createEmptyLlmModelConfig(): LlmModelConfig {
	return {
		id: "",
		modelType: "llm",
		model: "",
		modelIdentityHint: null,
		builtinPresetModelId: null,
		supportsMultimodal: false,
		supportsStateful: false,
	};
}

export function hydrateLlmProviderConfig(provider: LlmProviderConfig): LlmProviderConfig {
	const selectedModel =
		provider.models.find((model) => model.id === provider.modelConfig?.id) ??
		provider.models[0] ??
		provider.modelConfig ??
		createEmptyLlmModelConfig();

	return {
		...provider,
		modelConfig: selectedModel,
	};
}

export function hydrateLlmSettings(settings: LlmSettings): LlmSettings {
	return {
		...settings,
		providers: settings.providers.map(hydrateLlmProviderConfig),
	};
}

export function hydrateAppSettings(settings: AppSettings): AppSettings {
	return {
		...settings,
		llm: hydrateLlmSettings(settings.llm),
	};
}
