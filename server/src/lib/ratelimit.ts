// Rate limit por janela fixa, no D1 (poucas escritas: só login, convite e criação de upload).
import type { Context } from "hono";
import type { AppEnv } from "../env";

/** Incrementa e devolve `true` se ainda está dentro do limite. */
export async function allow(c: Context<AppEnv>, key: string, limit: number, windowSec: number) {
  const now = c.var.deps.now();
  const windowStart = Math.floor(now / (windowSec * 1000)) * windowSec * 1000;
  const row = await c.env.DB.prepare(
    `INSERT INTO rate_limits (key, window_start, count) VALUES (?, ?, 1)
     ON CONFLICT (key, window_start) DO UPDATE SET count = count + 1
     RETURNING count`,
  )
    .bind(key, windowStart)
    .first<{ count: number }>();
  return (row?.count ?? 1) <= limit;
}

export function clientIp(c: Context<AppEnv>): string {
  return c.req.header("CF-Connecting-IP") ?? "local";
}
