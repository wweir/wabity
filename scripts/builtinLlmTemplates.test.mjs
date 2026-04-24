import test from "node:test";
import assert from "node:assert/strict";
import { browserBuiltinLlmProviderTemplates } from "../src/lib/tauri/client/templates.ts";

test("includes newly added mainland provider templates with expected defaults", () => {
	const bailian = browserBuiltinLlmProviderTemplates.find((template) => template.id === "bailian");
	assert.ok(bailian);
	assert.equal(bailian.defaultBaseUrl, "https://dashscope.aliyuncs.com/compatible-mode/v1");
	assert.equal(bailian.supportsModelListing, false);
	assert.ok(bailian.models.some((model) => model.model === "text-embedding-v4"));

	const volcengine = browserBuiltinLlmProviderTemplates.find(
		(template) => template.id === "volcengine-ark",
	);
	assert.ok(volcengine);
	assert.equal(volcengine.defaultBaseUrl, "https://ark.cn-beijing.volces.com/api/v3");
	assert.equal(volcengine.supportsModelListing, false);
	assert.deepEqual(volcengine.models, []);

	const hunyuan = browserBuiltinLlmProviderTemplates.find(
		(template) => template.id === "tencent-hunyuan",
	);
	assert.ok(hunyuan);
	assert.equal(hunyuan.defaultBaseUrl, "https://api.hunyuan.cloud.tencent.com/v1");
	assert.equal(hunyuan.supportsModelListing, false);
	assert.ok(hunyuan.models.some((model) => model.model === "hunyuan-embedding"));
});
