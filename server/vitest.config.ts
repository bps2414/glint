import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig(async () => {
  const migrations = await readD1Migrations("./migrations");
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./wrangler.jsonc" },
        miniflare: {
          bindings: {
            TEST_MIGRATIONS: migrations,
            DISCORD_CLIENT_ID: "123456789",
            DISCORD_CLIENT_SECRET: "segredo-de-teste",
            SIGNING_KEY: "chave-de-teste-com-mais-de-32-caracteres-0123456789",
            ADMIN_DISCORD_IDS: "100000000000000001",
            SERVER_LANG: "pt-BR",
          },
        },
      }),
    ],
    test: { setupFiles: ["./test/setup.ts"] },
  };
});
