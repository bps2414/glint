/** @jsxImportSource hono/jsx */
// Painel do dono: convites, usuários, cotas. Só role = admin (ADMIN_DISCORD_IDS ou o primeiro login do servidor).
import { Hono, type Context } from "hono";
import { createInvite } from "./auth/invites";
import type { AppEnv } from "./env";
import { t } from "./i18n";
import { isToken } from "./lib/crypto";
import { totalBytes, userQuota } from "./lib/clips";
import { AdminPage, type InviteRow, type UserRow } from "./pages/admin";
import { Message, render } from "./pages/layout";

export const admin = new Hono<AppEnv>();

type Ctx = Context<AppEnv>;

admin.use("*", async (c, next) => {
  if (!c.var.user) return c.redirect(`/entrar?next=${encodeURIComponent("/admin")}`);
  if (c.var.user.role !== "admin") return render(c, <Message title={t.forbidden} text={t.forbidden} user={c.var.user} />, 403);
  await next();
});

async function page(c: Ctx, created?: string) {
  const db = c.env.DB;
  const [invites, users, total] = await Promise.all([
    db.prepare("SELECT * FROM invites ORDER BY created_at DESC LIMIT 100").all<InviteRow>(),
    db
      .prepare(
        `SELECT u.*, COALESCE(SUM(c.size_bytes + c.thumb_bytes), 0) AS used, COUNT(c.id) AS clips
           FROM users u LEFT JOIN clips c ON c.user_id = u.id
          GROUP BY u.id ORDER BY u.created_at`,
      )
      .all<UserRow>(),
    totalBytes(db),
  ]);
  const config = c.var.config;
  return render(
    c,
    <AdminPage
      me={c.var.user!}
      invites={invites.results}
      users={users.results.map((u) => ({ ...u, quota_bytes: userQuota(config, u) }))}
      now={c.var.deps.now()}
      created={created}
      quotaOn={config.userQuotaBytes > 0}
      total={total}
      limit={config.storageLimitBytes}
    />,
  );
}

admin.get("/", (c) => page(c));

admin.post("/convites", async (c) => {
  const f = await c.req.parseBody();
  const note = typeof f.note === "string" ? f.note.trim().replace(/[\u0000-\u001f\u007f]/g, "").slice(0, 60) : "";
  const maxUses = clampInt(f.max_uses, 1, 100, 1);
  const days = clampInt(f.days, 0, 365, 7);
  const now = c.var.deps.now();
  const { code } = await createInvite(
    c.env.DB,
    c.var.user!.id,
    { note: note || null, maxUses, expiresAt: days > 0 ? now + days * 86400_000 : null },
    now,
  );
  return page(c, `${new URL(c.req.url).origin}/i/${code}`);
});

admin.post("/convites/:id/revogar", async (c) => {
  const id = c.req.param("id");
  if (isToken(id, 22, 22)) {
    await c.env.DB.prepare("UPDATE invites SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
      .bind(c.var.deps.now(), id)
      .run();
  }
  return c.redirect("/admin", 303);
});

admin.post("/usuarios/:id/cota", async (c) => {
  const id = c.req.param("id");
  const f = await c.req.parseBody();
  const gb = Number(f.quota_gb);
  if (isToken(id, 22, 22) && Number.isFinite(gb) && gb >= 0 && gb <= 10000) {
    await c.env.DB.prepare("UPDATE users SET quota_bytes = ? WHERE id = ?")
      .bind(Math.round(gb * 1024 ** 3), id)
      .run();
  }
  return c.redirect("/admin", 303);
});

for (const [action, disabled] of [
  ["desativar", 1],
  ["reativar", 0],
] as const) {
  admin.post(`/usuarios/:id/${action}`, async (c) => {
    const id = c.req.param("id");
    if (isToken(id, 22, 22) && id !== c.var.user!.id) {
      const stmts = [c.env.DB.prepare("UPDATE users SET disabled = ? WHERE id = ?").bind(disabled, id)];
      // Desativar derruba as sessões (web e app) na hora.
      if (disabled) stmts.push(c.env.DB.prepare("DELETE FROM sessions WHERE user_id = ?").bind(id));
      await c.env.DB.batch(stmts);
    }
    return c.redirect("/admin", 303);
  });
}

function clampInt(v: unknown, min: number, max: number, fallback: number): number {
  const n = typeof v === "string" ? Number.parseInt(v, 10) : Number.NaN;
  return Number.isInteger(n) ? Math.min(max, Math.max(min, n)) : fallback;
}
