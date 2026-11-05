import { applyD1Migrations, env } from "cloudflare:test";
import { beforeEach } from "vitest";
import { clock, vars } from "./helpers";

await applyD1Migrations(env.DB, env.TEST_MIGRATIONS);

const START = Date.UTC(2026, 10, 4, 12);

// O armazenamento do Miniflare é compartilhado entre testes do arquivo: começa limpo.
beforeEach(async () => {
  clock.now = START;
  for (const k of Object.keys(vars)) delete vars[k];
  await env.DB.batch(
    ["clip_parts", "clips", "sessions", "auth_flows", "rate_limits", "invites", "users"].map((t) =>
      env.DB.prepare(`DELETE FROM ${t}`),
    ),
  );
  const listed = await env.BUCKET.list();
  if (listed.objects.length) await env.BUCKET.delete(listed.objects.map((o) => o.key));
});
