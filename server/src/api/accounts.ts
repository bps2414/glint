// API v1 de contas: servidor, entrar por convite, criar o admin, convites, membros e aparelhos.
// Montada antes de `api` (clipes), que exige login em tudo; aqui cada rota diz o que exige.
// Contrato: docs/API.md.
import { Hono, type Context, type MiddlewareHandler } from "hono";
import { z } from "zod";
import { avatarUrl } from "../auth/discord";
import { createInvite, parseInviteCode, redeemInvite, refundInvite } from "../auth/invites";
import { createSession, deviceId, putFlow, takeFlow } from "../auth/session";
import type { AppEnv, User } from "../env";
import { API_VERSION, LINK_CODE_MS, SERVER_VERSION, WEB_CODE_MS } from "../env";
import { t } from "../i18n";
import { isToken, randomId, safeEqual, sha256 } from "../lib/crypto";
import { allow, clientIp } from "../lib/ratelimit";
import { safeNext } from "../lib/security";
import { apiError } from "./clips";

type Ctx = Context<AppEnv>;

export const accounts = new Hono<AppEnv>();

/** Usuário como a API devolve (`/me`, `join`, `setup`). `username` fica pelo app antigo. */
export function userDto(u: Pick<User, "id" | "discord_id" | "name" | "username" | "display_name" | "avatar" | "role">) {
  return {
    id: u.id,
    name: u.name,
    username: u.username,
    display_name: u.display_name,
    avatar_url: u.discord_id ? avatarUrl(u.discord_id, u.avatar) : null,
    role: u.role,
  };
}

const signedIn: MiddlewareHandler<AppEnv> = async (c, next) => {
  if (!c.var.user) return apiError(c, 401, "unauthorized", t.api.unauthorized);
  await next();
};

const adminOnly: MiddlewareHandler<AppEnv> = async (c, next) => {
  if (!c.var.user) return apiError(c, 401, "unauthorized", t.api.unauthorized);
  if (c.var.user.role !== "admin") return apiError(c, 403, "forbidden", t.api.forbidden);
  await next();
};

/** Nome escolhido pela pessoa: 1–32 caracteres, sem caracteres de controle. */
const personName = z
  .string()
  .trim()
  .min(1)
  .max(32)
  .refine((s) => !/[\u0000-\u001f\u007f]/.test(s));

async function body(c: Ctx): Promise<unknown> {
  return c.req.json().catch(() => undefined);
}

/** Conta nova sem Discord + token de aparelho (app). */
async function newAccount(c: Ctx, name: string, role: "user" | "admin", inviteId: string | null) {
  const id = randomId();
  const now = c.var.deps.now();
  const sql = `INSERT INTO users (id, discord_id, name, username, role, quota_bytes, invite_id, created_at, last_login_at)
               SELECT ?, NULL, ?, ?, ?, ?, ?, ?, ?`;
  // O admin do servidor novo só nasce com a tabela vazia (duas chamadas juntas não criam dois).
  const res = await c.env.DB.prepare(role === "admin" ? `${sql} WHERE NOT EXISTS (SELECT 1 FROM users)` : sql)
    .bind(id, name, name, role, c.var.config.userQuotaBytes, inviteId, now, now)
    .run();
  if (!res.meta.changes) return null;
  return issueAppToken(c, id);
}

async function issueAppToken(c: Ctx, userId: string) {
  const user = (await c.env.DB.prepare("SELECT * FROM users WHERE id = ?").bind(userId).first<User>())!;
  const s = await createSession(c, userId, "app");
  return { token: s.token, expires_at: s.expires_at, user: userDto(user) };
}

// --- público ------------------------------------------------------------------

accounts.get("/info", async (c) => {
  const any = await c.env.DB.prepare("SELECT 1 AS x FROM users LIMIT 1").first();
  return c.json({
    name: c.var.config.serverName,
    server_version: SERVER_VERSION,
    api_version: API_VERSION,
    discord_login: c.var.config.discordLogin,
    // Servidor sem usuários esperando o POST /setup (só quando SETUP_TOKEN está configurado).
    setup_required: !any && !!c.env.SETUP_TOKEN,
  });
});

const joinSchema = z.object({ invite: z.string().max(300), name: personName });

accounts.post("/join", async (c) => {
  if (!(await allow(c, `join:${clientIp(c)}`, 10, 600))) return apiError(c, 429, "rate_limited", t.api.tooMany);
  const parsed = joinSchema.safeParse(await body(c));
  if (!parsed.success) return apiError(c, 400, "invalid", t.api.invalid);
  const code = parseInviteCode(parsed.data.invite);
  const inviteId = code ? await redeemInvite(c.env.DB, code, c.var.deps.now()) : null;
  if (!inviteId) return apiError(c, 400, "invalid_invite", t.inviteInvalid);
  try {
    return c.json((await newAccount(c, parsed.data.name, "user", inviteId))!, 201);
  } catch (e) {
    await refundInvite(c.env.DB, inviteId);
    throw e;
  }
});

const setupSchema = z.object({ setup_token: z.string().min(1).max(256), name: personName });

accounts.post("/setup", async (c) => {
  if (!(await allow(c, `setup:${clientIp(c)}`, 10, 600))) return apiError(c, 429, "rate_limited", t.api.tooMany);
  const secret = c.env.SETUP_TOKEN ?? "";
  if (secret.length < 16) return apiError(c, 404, "not_found", t.api.notFound);
  const parsed = setupSchema.safeParse(await body(c));
  if (!parsed.success) return apiError(c, 400, "invalid", t.api.invalid);
  // Compara os hashes: tempo constante mesmo com tamanhos diferentes.
  if (!safeEqual(await sha256(parsed.data.setup_token), await sha256(secret))) {
    return apiError(c, 403, "forbidden", t.api.forbidden);
  }
  const res = await newAccount(c, parsed.data.name, "admin", null);
  if (!res) return apiError(c, 409, "already_setup", t.api.alreadySetup);
  return c.json(res, 201);
});

// --- entrar em outro PC (código curto) ------------------------------------------

/** Sem 0/O, 1/I: 32 símbolos, 8 caracteres = 40 bits, vale 10 minutos, uma vez só. */
const LINK_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

const normalizeLinkCode = (s: unknown) => (typeof s === "string" ? s.toUpperCase().replace(/[^A-Z0-9]/g, "") : "");

accounts.post("/devices/link", signedIn, async (c) => {
  if (!(await allow(c, `linkcode:${c.var.user!.id}`, 10, 600))) return apiError(c, 429, "rate_limited", t.api.tooMany);
  const rnd = crypto.getRandomValues(new Uint8Array(8));
  const code = [...rnd].map((b) => LINK_ALPHABET[b % 32]).join("");
  await putFlow(c, "link", { user_id: c.var.user!.id }, LINK_CODE_MS, code);
  return c.json({ code: `${code.slice(0, 4)}-${code.slice(4)}`, expires_at: c.var.deps.now() + LINK_CODE_MS }, 201);
});

accounts.post("/devices/link/claim", async (c) => {
  if (!(await allow(c, `link:${clientIp(c)}`, 10, 600))) return apiError(c, 429, "rate_limited", t.api.tooMany);
  const b = (await body(c)) as { code?: unknown } | undefined;
  const code = normalizeLinkCode(b?.code);
  const flow = code.length === 8 ? await takeFlow<{ user_id: string }>(c, "link", code, 8) : null;
  const user = flow
    ? await c.env.DB.prepare("SELECT id FROM users WHERE id = ? AND disabled = 0").bind(flow.user_id).first<{ id: string }>()
    : null;
  if (!user) return apiError(c, 400, "invalid_code", t.api.badLinkCode);
  return c.json(await issueAppToken(c, user.id), 201);
});

// --- site já logado -------------------------------------------------------------

accounts.post("/auth/web-code", signedIn, async (c) => {
  if (!(await allow(c, `webcode:${c.var.user!.id}`, 30, 600))) return apiError(c, 429, "rate_limited", t.api.tooMany);
  const b = (await body(c)) as { next?: unknown } | undefined;
  const next = safeNext(typeof b?.next === "string" ? b.next : "/");
  const code = await putFlow(c, "web_code", { user_id: c.var.user!.id, next }, WEB_CODE_MS);
  const url = `${new URL(c.req.url).origin}/auth/web?code=${code}`;
  return c.json({ url, expires_at: c.var.deps.now() + WEB_CODE_MS });
});

// --- admin ----------------------------------------------------------------------

const inviteSchema = z.object({
  note: z
    .string()
    .trim()
    .max(60)
    .refine((s) => !/[\u0000-\u001f\u007f]/.test(s))
    .nullish(),
  max_uses: z.int().min(1).max(100).default(1),
  days: z.int().min(0).max(365).default(7),
});

accounts.post("/invites", adminOnly, async (c) => {
  const parsed = inviteSchema.safeParse((await body(c)) ?? {});
  if (!parsed.success) return apiError(c, 400, "invalid", t.api.invalid);
  const { note, max_uses, days } = parsed.data;
  const now = c.var.deps.now();
  const expiresAt = days > 0 ? now + days * 86400_000 : null;
  const { id, code } = await createInvite(c.env.DB, c.var.user!.id, { note: note || null, maxUses: max_uses, expiresAt }, now);
  return c.json(
    {
      invite: { id, url: `${new URL(c.req.url).origin}/i/${code}`, note: note || null, max_uses, expires_at: expiresAt },
    },
    201,
  );
});

accounts.get("/members", adminOnly, async (c) => {
  const db = c.env.DB;
  const now = c.var.deps.now();
  const [users, sessions] = await Promise.all([
    db
      .prepare(
        `SELECT u.*, COALESCE(SUM(c.size_bytes + c.thumb_bytes), 0) AS used_bytes, COUNT(c.id) AS clips
           FROM users u LEFT JOIN clips c ON c.user_id = u.id
          GROUP BY u.id ORDER BY u.created_at`,
      )
      .all<User & { used_bytes: number; clips: number; last_login_at: number }>(),
    db
      .prepare(
        "SELECT token_hash, user_id, kind, created_at, last_used_at, expires_at FROM sessions WHERE expires_at > ? ORDER BY last_used_at DESC",
      )
      .bind(now)
      .all<{ token_hash: string; user_id: string; kind: string; created_at: number; last_used_at: number; expires_at: number }>(),
  ]);
  const current = c.var.session?.token_hash;
  const members = users.results.map((u) => ({
    ...userDto(u),
    discord: u.discord_id !== null,
    disabled: !!u.disabled,
    created_at: u.created_at,
    last_login_at: u.last_login_at,
    used_bytes: u.used_bytes,
    clips: u.clips,
    devices: sessions.results
      .filter((s) => s.user_id === u.id)
      .map((s) => ({
        id: deviceId(s.token_hash),
        kind: s.kind,
        created_at: s.created_at,
        last_used_at: s.last_used_at,
        expires_at: s.expires_at,
        current: s.token_hash === current,
      })),
  }));
  return c.json({ members });
});

for (const [action, disabled] of [
  ["disable", 1],
  ["enable", 0],
] as const) {
  accounts.post(`/members/:id/${action}`, adminOnly, async (c) => {
    const id = c.req.param("id");
    if (!isToken(id, 22, 22)) return apiError(c, 404, "not_found", t.api.notFound);
    if (id === c.var.user!.id) return apiError(c, 409, "self", t.api.notSelf);
    const stmts = [c.env.DB.prepare("UPDATE users SET disabled = ? WHERE id = ?").bind(disabled, id)];
    // Desativar derruba as sessões (web e app) na hora.
    if (disabled) stmts.push(c.env.DB.prepare("DELETE FROM sessions WHERE user_id = ?").bind(id));
    const [res] = await c.env.DB.batch(stmts);
    if (!res!.meta.changes) return apiError(c, 404, "not_found", t.api.notFound);
    return c.body(null, 204);
  });
}

accounts.delete("/members/:id/devices/:device", adminOnly, async (c) => {
  const { id, device } = c.req.param();
  if (!isToken(id, 22, 22) || !isToken(device, 22, 22)) return apiError(c, 404, "not_found", t.api.notFound);
  const res = await c.env.DB.prepare("DELETE FROM sessions WHERE user_id = ? AND substr(token_hash, 1, 22) = ?")
    .bind(id, device)
    .run();
  if (!res.meta.changes) return apiError(c, 404, "not_found", t.api.notFound);
  return c.body(null, 204);
});
