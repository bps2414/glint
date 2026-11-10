// Rodar a partir de server/: `npx vitest run --config feedback/vitest.config.ts` (ou `npm test`).
import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig(async () => {
  const migrations = await readD1Migrations("./feedback/migrations");
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./feedback/wrangler.jsonc" },
        miniflare: { bindings: { TEST_MIGRATIONS: migrations, ADMIN_TOKEN: "admin-de-teste" } },
      }),
    ],
    test: { include: ["feedback/test/**/*.test.ts"] },
  };
});
