// Sessões (cookie na web, Bearer no app) e etapas curtas do login (auth_flows).
import type { Context, MiddlewareHandler } from "hono";
import { deleteCookie, getCookie, setCookie } from "hono/cookie";
import type { AppEnv, Session, User } from "../env";
import { APP_SESSION_MS, WEB_SESSION_MS } from "../env";
import { isToken, randomToken, sha256 } from "../lib/crypto";

export const SESSION_COOKIE = "sess";

type Ctx = Context<AppEnv>;

/** Cookie `__Host-`: só HTTPS, sem Domain, Path=/ — não vaza para subdomínios. */
export function setHostCookie(c: Ctx, name: string, value: string, maxAgeSec: number) {
  setCookie(c, name, value, {
    prefix: "host",
    path: "/",
    secure: true,
    httpOnly: true,
    sameSite: "Lax",
    maxAge: maxAgeSec,
  });
}

export function getHostCookie(c: Ctx, name: string): string | undefined {
  return getCookie(c, name, "host");
}

export function clearHostCookie(c: Ctx, name: string) {
  deleteCookie(c, name, { prefix: "host", path: "/", secure: true });
}

export async function createSession(c: Ctx, userId: string, kind: "web" | "app") {
  const token = randomToken(32);
  const now = c.var.deps.now();
  const expires = now + (kind === "web" ? WEB_SESSION_MS : APP_SESSION_MS);
  await c.env.DB.prepare(
    "INSERT INTO sessions (token_hash, user_id, kind, created_at, expires_at, last_used_at) VALUES (?, ?, ?, ?, ?, ?)",
  )
    .bind(await sha256(token), userId, kind, now, expires, now)
    .run();
  if (kind === "web") setHostCookie(c, SESSION_COOKIE, token, Math.floor(WEB_SESSION_MS / 1000));
  return { token, expires_at: expires };
}

/** Carrega usuário/sessão (se houver) em `c.var`. Não exige login. */
export const loadSession: MiddlewareHandler<AppEnv> = async (c, next) => {
  c.set("user", null);
  c.set("session", null);
  c.set("authVia", null);

  let token: string | undefined;
  let via: "cookie" | "bearer" | null = null;
  const auth = c.req.header("Authorization");
  if (auth) {
    const m = /^Bearer ([A-Za-z0-9_-]{20,128})$/.exec(auth);
    if (m) {
      token = m[1];
      via = "bearer";
    }
  } else {
    const ck = getHostCookie(c, SESSION_COOKIE);
    if (isToken(ck, 20)) {
      token = ck;
      via = "cookie";
    }
  }

  if (token) {
    const now = c.var.deps.now();
    const row = await c.env.DB.prepare(
      `SELECT s.token_hash, s.kind, s.expires_at, s.last_used_at,
              u.id, u.discord_id, u.name, u.username, u.display_name, u.avatar, u.role,
              u.quota_bytes, u.disabled, u.created_at
         FROM sessions s JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = ? AND s.expires_at > ? AND u.disabled = 0`,
    )
      .bind(await sha256(token), now)
      .first<Session & User>();
    // Token de app não vale como cookie e vice-versa.
    if (row && (row.kind === "app") === (via === "bearer")) {
      const { token_hash, kind, expires_at, last_used_at, ...user } = row;
      c.set("user", user);
      c.set("session", { token_hash, kind, expires_at, last_used_at });
      c.set("authVia", via);
      if (now - last_used_at > 3600_000) {
        c.executionCtx.waitUntil(
          c.env.DB.prepare("UPDATE sessions SET last_used_at = ? WHERE token_hash = ?")
            .bind(now, token_hash)
            .run(),
        );
      }
    }
  }
  await next();
};

/** ID público de um aparelho (sessão): começo do hash do token, nunca o token. */
export const deviceId = (tokenHash: string) => tokenHash.slice(0, 22);

export async function destroySession(c: Ctx) {
  const s = c.var.session;
  if (s) await c.env.DB.prepare("DELETE FROM sessions WHERE token_hash = ?").bind(s.token_hash).run();
  if (c.var.authVia === "cookie" || !c.var.authVia) clearHostCookie(c, SESSION_COOKIE);
}

// ---------------------------------------------------------------------------
// auth_flows: linhas de uso único, chaveadas pelo hash de um token aleatório.

export type FlowKind = "oauth" | "signup" | "app_code" | "link" | "web_code";

/** `token` dado: usa ele (ex.: código curto de outro PC) em vez de um aleatório de 256 bits. */
export async function putFlow(c: Ctx, kind: FlowKind, data: unknown, ttlMs: number, token = randomToken(32)): Promise<string> {
  await c.env.DB.prepare("INSERT INTO auth_flows (id_hash, kind, data, expires_at) VALUES (?, ?, ?, ?)")
    .bind(await sha256(token), kind, JSON.stringify(data), c.var.deps.now() + ttlMs)
    .run();
  return token;
}

/** Lê e apaga (uso único). `null` se não existe, venceu ou é de outro tipo. */
export async function takeFlow<T>(c: Ctx, kind: FlowKind, token: string | undefined, minLen = 20): Promise<T | null> {
  if (!isToken(token, minLen)) return null;
  const row = await c.env.DB.prepare(
    "DELETE FROM auth_flows WHERE id_hash = ? AND kind = ? RETURNING data, expires_at",
  )
    .bind(await sha256(token), kind)
    .first<{ data: string; expires_at: number }>();
  if (!row || row.expires_at <= c.var.deps.now()) return null;
  return JSON.parse(row.data) as T;
}

/** Lê sem apagar (ex.: mostrar o formulário de convite). */
export async function peekFlow<T>(c: Ctx, kind: FlowKind, token: string | undefined): Promise<T | null> {
  if (!isToken(token, 20)) return null;
  const row = await c.env.DB.prepare("SELECT data, expires_at FROM auth_flows WHERE id_hash = ? AND kind = ?")
    .bind(await sha256(token), kind)
    .first<{ data: string; expires_at: number }>();
  if (!row || row.expires_at <= c.var.deps.now()) return null;
  return JSON.parse(row.data) as T;
}
