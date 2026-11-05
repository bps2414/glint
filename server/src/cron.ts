// Faxina diária (cron do wrangler.jsonc).
import { deleteClip } from "./api/clips";
import type { Bindings } from "./env";
import { UPLOAD_TTL_MS } from "./env";
import type { ClipRow } from "./lib/clips";

export async function cleanup(env: Bindings, now: number) {
  // Uploads abandonados: aborta o multipart, apaga e libera a cota.
  const { results } = await env.DB.prepare(
    "SELECT * FROM clips WHERE status = 'uploading' AND created_at < ? LIMIT 200",
  )
    .bind(now - UPLOAD_TTL_MS)
    .all<ClipRow>();
  for (const clip of results) await deleteClip(env, clip);

  await env.DB.batch([
    env.DB.prepare("DELETE FROM sessions WHERE expires_at <= ?").bind(now),
    env.DB.prepare("DELETE FROM auth_flows WHERE expires_at <= ?").bind(now),
    env.DB.prepare("DELETE FROM rate_limits WHERE window_start < ?").bind(now - 86400_000),
  ]);
  return { abandonedUploads: results.length };
}
