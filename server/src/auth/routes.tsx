/** @jsxImportSource hono/jsx */
// Login: Discord OAuth → (convite, se for conta nova) → sessão web ou código para o app.
import { Hono, type Context } from "hono";
import type { AppEnv, User } from "../env";
import { t } from "../i18n";
import { isToken, randomId, safeEqual, sha256 } from "../lib/crypto";
import { allow, clientIp } from "../lib/ratelimit";
import { safeNext } from "../lib/security";
import { InvitePage } from "../pages/invite";
import { Message, render } from "../pages/layout";
import type { DiscordProfile } from "./discord";
import { parseInviteCode, redeemInvite, refundInvite } from "./invites";
import {
  clearHostCookie,
  createSession,
  destroySession,
  getHostCookie,
  peekFlow,
  putFlow,
  setHostCookie,
  takeFlow,
} from "./session";

type Ctx = Context<AppEnv>;

/** Para onde o login volta: navegador (sessão por cookie) ou app (loopback + PKCE). */
type OAuthFlow =
  | { mode: "web"; next: string }
  | { mode: "app"; port: number; challenge: string; state: string };
type SignupFlow = { profile: DiscordProfile; flow: OAuthFlow };
type AppCodeFlow = { user_id: string; challenge: string };

const OAUTH_COOKIE = "oauth";
const INVITE_COOKIE = "invite";
const SIGNUP_COOKIE = "signup";
const OAUTH_TTL = 10 * 60_000;
const SIGNUP_TTL = 15 * 60_000;
const APP_CODE_TTL = 2 * 60_000;

export const authRoutes = new Hono<AppEnv>();

function tooMany(c: Ctx) {
  return render(c, <Message title={t.tooMany} text={t.tooMany} />, 429);
}

async function startOAuth(c: Ctx, flow: OAuthFlow) {
  if (!(await allow(c, `login:${clientIp(c)}`, 30, 600))) return tooMany(c);
  if (!c.env.DISCORD_CLIENT_ID) throw new Error("DISCORD_CLIENT_ID não configurado");
  const state = await putFlow(c, "oauth", flow, OAUTH_TTL);
  // O state também fica num cookie: o callback só vale no mesmo navegador (anti login-CSRF).
  setHostCookie(c, OAUTH_COOKIE, state, OAUTH_TTL / 1000);
  return c.redirect(
    c.var.deps.discord.authorizeUrl({
      clientId: c.env.DISCORD_CLIENT_ID,
      redirectUri: redirectUri(c),
      state,
    }),
  );
}

const redirectUri = (c: Ctx) => `${new URL(c.req.url).origin}/auth/discord/callback`;

authRoutes.get("/entrar", async (c) => {
  const next = safeNext(c.req.query("next"));
  if (c.var.user) return c.redirect(next);
  return startOAuth(c, { mode: "web", next });
});

/**
 * Login do app (RFC 8252): o app escuta em 127.0.0.1:<port>, abre esta URL no navegador com
 * `challenge` = base64url(SHA-256(verifier)) e recebe `?code&state` no loopback. O código só
 * vira token com o verifier (POST /api/v1/auth/token), então outro programa que veja o
 * redirect não consegue usá-lo.
 */
authRoutes.get("/auth/app", async (c) => {
  const port = Number(c.req.query("port"));
  const challenge = c.req.query("challenge");
  const state = c.req.query("state");
  if (!Number.isInteger(port) || port < 1024 || port > 65535 || !isToken(challenge, 43, 43) || !isToken(state, 8, 128)) {
    return render(c, <Message title={t.loginFailed} text={t.api.invalid} />, 400);
  }
  return startOAuth(c, { mode: "app", port, challenge, state });
});

authRoutes.get("/auth/discord/callback", async (c) => {
  const cookieState = getHostCookie(c, OAUTH_COOKIE);
  clearHostCookie(c, OAUTH_COOKIE);
  const state = c.req.query("state");
  if (!state || !cookieState || !safeEqual(state, cookieState)) {
    return render(c, <Message title={t.loginFailed} text={t.inviteExpiredSession} />, 400);
  }
  const flow = await takeFlow<OAuthFlow>(c, "oauth", state);
  if (!flow) return render(c, <Message title={t.loginFailed} text={t.inviteExpiredSession} />, 400);

  const code = c.req.query("code");
  if (c.req.query("error") || !code) {
    return render(c, <Message title={t.loginCancelled} text={t.loginFailed} />, 400);
  }

  let profile: DiscordProfile;
  try {
    profile = await c.var.deps.discord.profile({
      clientId: c.env.DISCORD_CLIENT_ID,
      clientSecret: c.env.DISCORD_CLIENT_SECRET,
      redirectUri: redirectUri(c),
      code: code.slice(0, 200),
    });
  } catch (e) {
    console.error("discord oauth", e);
    return render(c, <Message title={t.loginFailed} text={t.loginFailed} />, 400);
  }

  const now = c.var.deps.now();
  const existing = await c.env.DB.prepare("SELECT * FROM users WHERE discord_id = ?")
    .bind(profile.id)
    .first<User>();
  if (existing) {
    if (existing.disabled) return render(c, <Message title={t.forbidden} text={t.disabled} />, 403);
    // Só promove: quem virou admin por ser o primeiro usuário continua admin.
    const role = roleFor(c, profile.id) === "admin" ? "admin" : existing.role;
    await c.env.DB.prepare(
      "UPDATE users SET username = ?, display_name = ?, avatar = ?, role = ?, last_login_at = ? WHERE id = ?",
    )
      .bind(profile.username, profile.global_name, profile.avatar, role, now, existing.id)
      .run();
    return finishLogin(c, flow, existing.id);
  }

  const isAdmin = c.var.config.adminDiscordIds.has(profile.id);
  // Servidor novo (sem usuários e sem ADMIN_DISCORD_IDS): o primeiro login vira admin, sem convite.
  if (!isAdmin && c.var.config.adminDiscordIds.size === 0) {
    const firstId = await createFirstAdmin(c, profile);
    if (firstId) return finishLogin(c, flow, firstId);
  }
  if (isAdmin || !c.var.config.inviteRequired) {
    return finishLogin(c, flow, await createUser(c, profile, null));
  }

  // Conta nova e convite obrigatório: tenta o convite do link (/i/<código>), senão pede o código.
  const fromLink = parseInviteCode(getHostCookie(c, INVITE_COOKIE));
  clearHostCookie(c, INVITE_COOKIE);
  if (fromLink) {
    const userId = await signupWithInvite(c, profile, fromLink);
    if (userId) return finishLogin(c, flow, userId);
  }
  const signup = await putFlow(c, "signup", { profile, flow } satisfies SignupFlow, SIGNUP_TTL);
  setHostCookie(c, SIGNUP_COOKIE, signup, SIGNUP_TTL / 1000);
  return c.redirect(fromLink ? "/convite?erro=1" : "/convite");
});

/** Link de convite: guarda o código e manda para o login. */
authRoutes.get("/i/:code", async (c) => {
  const code = parseInviteCode(c.req.param("code"));
  if (!code) return render(c, <Message title={t.inviteTitle} text={t.inviteInvalid} />, 404);
  if (c.var.user) return c.redirect("/");
  setHostCookie(c, INVITE_COOKIE, code, 3600);
  return c.redirect("/entrar");
});

authRoutes.get("/convite", async (c) => {
  const signup = await peekFlow<SignupFlow>(c, "signup", getHostCookie(c, SIGNUP_COOKIE));
  if (!signup) {
    return render(
      c,
      <Message
        title={t.inviteTitle}
        text={t.inviteExpiredSession}
        action={
          <a class="btn" href="/entrar">
            {t.login}
          </a>
        }
      />,
      400,
    );
  }
  const name = signup.profile.global_name ?? signup.profile.username;
  return render(c, <InvitePage name={name} error={c.req.query("erro") ? t.inviteInvalid : undefined} />);
});

authRoutes.post("/convite", async (c) => {
  if (!(await allow(c, `convite:${clientIp(c)}`, 10, 600))) return tooMany(c);
  const token = getHostCookie(c, SIGNUP_COOKIE);
  const signup = await peekFlow<SignupFlow>(c, "signup", token);
  if (!signup) return render(c, <Message title={t.inviteTitle} text={t.inviteExpiredSession} />, 400);

  const form = await c.req.parseBody();
  const code = parseInviteCode(typeof form.code === "string" ? form.code : null);
  const name = signup.profile.global_name ?? signup.profile.username;
  const userId = code ? await signupWithInvite(c, signup.profile, code) : null;
  if (!userId) return render(c, <InvitePage name={name} error={t.inviteInvalid} />, 400);

  await takeFlow(c, "signup", token);
  clearHostCookie(c, SIGNUP_COOKIE);
  return finishLogin(c, signup.flow, userId, 303);
});

authRoutes.post("/sair", async (c) => {
  await destroySession(c);
  return c.redirect("/", 303);
});

// --- API do app -------------------------------------------------------------

authRoutes.post("/api/v1/auth/token", async (c) => {
  if (!(await allow(c, `token:${clientIp(c)}`, 20, 600))) {
    return c.json({ error: { code: "rate_limited", message: t.api.tooMany } }, 429);
  }
  const body = (await c.req.json().catch(() => null)) as { code?: unknown; verifier?: unknown } | null;
  const verifier = body?.verifier;
  const flow = await takeFlow<AppCodeFlow>(c, "app_code", typeof body?.code === "string" ? body.code : undefined);
  if (!flow || !isToken(verifier, 43, 128) || !safeEqual(await sha256(verifier), flow.challenge)) {
    return c.json({ error: { code: "invalid_code", message: t.api.badCode } }, 400);
  }
  const user = await c.env.DB.prepare("SELECT * FROM users WHERE id = ? AND disabled = 0")
    .bind(flow.user_id)
    .first<User>();
  if (!user) return c.json({ error: { code: "invalid_code", message: t.api.badCode } }, 400);
  const s = await createSession(c, user.id, "app");
  return c.json({ token: s.token, expires_at: s.expires_at });
});

authRoutes.post("/api/v1/auth/logout", async (c) => {
  await destroySession(c);
  return c.body(null, 204);
});

// --- auxiliares -------------------------------------------------------------

function roleFor(c: Ctx, discordId: string): "admin" | "user" {
  return c.var.config.adminDiscordIds.has(discordId) ? "admin" : "user";
}

async function createUser(c: Ctx, p: DiscordProfile, inviteId: string | null): Promise<string> {
  const id = randomId();
  const now = c.var.deps.now();
  await c.env.DB.prepare(
    `INSERT INTO users (id, discord_id, username, display_name, avatar, role, quota_bytes, invite_id, created_at, last_login_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  )
    .bind(id, p.id, p.username, p.global_name, p.avatar, roleFor(c, p.id), c.var.config.userQuotaBytes, inviteId, now, now)
    .run();
  return id;
}

/** Cria o admin do servidor novo, de forma atômica: só entra se a tabela de usuários estiver vazia. */
async function createFirstAdmin(c: Ctx, p: DiscordProfile): Promise<string | null> {
  const id = randomId();
  const now = c.var.deps.now();
  const res = await c.env.DB.prepare(
    `INSERT INTO users (id, discord_id, username, display_name, avatar, role, quota_bytes, invite_id, created_at, last_login_at)
     SELECT ?, ?, ?, ?, ?, 'admin', ?, NULL, ?, ?
      WHERE NOT EXISTS (SELECT 1 FROM users)`,
  )
    .bind(id, p.id, p.username, p.global_name, p.avatar, c.var.config.userQuotaBytes, now, now)
    .run();
  return res.meta.changes ? id : null;
}

async function signupWithInvite(c: Ctx, p: DiscordProfile, code: string): Promise<string | null> {
  const db = c.env.DB;
  const inviteId = await redeemInvite(db, code, c.var.deps.now());
  if (!inviteId) return null;
  try {
    return await createUser(c, p, inviteId);
  } catch (e) {
    // Corrida (duas abas): a conta já foi criada; devolve o uso e entra nela.
    await refundInvite(db, inviteId);
    const u = await db.prepare("SELECT id FROM users WHERE discord_id = ?").bind(p.id).first<{ id: string }>();
    if (u) return u.id;
    throw e;
  }
}

async function finishLogin(c: Ctx, flow: OAuthFlow, userId: string, status: 302 | 303 = 302) {
  if (flow.mode === "web") {
    await createSession(c, userId, "web");
    return c.redirect(flow.next, status);
  }
  const code = await putFlow(c, "app_code", { user_id: userId, challenge: flow.challenge } satisfies AppCodeFlow, APP_CODE_TTL);
  const u = new URL(`http://127.0.0.1:${flow.port}/callback`);
  u.searchParams.set("code", code);
  u.searchParams.set("state", flow.state);
  return c.redirect(u.toString(), status);
}
