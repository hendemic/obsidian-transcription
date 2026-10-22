import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

export default defineConfig({
	test: {
		include: ["src/**/*.test.ts"],
		environment: "node",
	},
	resolve: {
		// The `obsidian` package ships types only; tests import this mock instead.
		alias: {
			obsidian: fileURLToPath(new URL("./src/__mocks__/obsidian.ts", import.meta.url)),
		},
	},
});
