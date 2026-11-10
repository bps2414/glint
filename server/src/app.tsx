/** @jsxImportSource hono/jsx */
import { Hono } from "hono";
import { admin } from "./admin";
import { accounts } from "./api/accounts";
import { api, apiError } from "./api/clips";
import { authRoutes } from "./auth/routes";
import { loadSession } from "./auth/session";
import type { AppEnv, Deps, User } from "./env";
import { readConfig } from "./env";
import { setLang, t } from "./i18n";
import { clipUrls, isClipId, usedBytes, userQuota, type ClipRow } from "./lib/clips";
import { sameOriginForCookies, securityHeaders } from "./lib/security";
import { media } from "./media";
import { releases } from "./releases";
import { ClipPage } from "./pages/clip";
import { HomeLoggedIn, HomeLoggedOut } from "./pages/home";
import { Message, render, setDiscordLogin } from "./pages/layout";
import { UploadPage } from "./pages/upload";

export function createApp(deps: Deps) {
  const app = new Hono<AppEnv>();

  app.use("*", securityHeaders);
  app.use("*", async (c, next) => {
    c.set("deps", deps);
    c.set("config", readConfig(c.env));
    setLang(c.env.SERVER_LANG);
    setDiscordLogin(c.var.config.discordLogin);
    await next();
  });
  app.use("*", sameOriginForCookies);
  app.use("*", loadSession);

  app.get("/", async (c) => {
    const user = c.var.user;
    if (!user) return render(c, <HomeLoggedOut notice={c.req.query("convite") ? t.inviteLinkSaved : undefined} />);
    const { results } = await c.env.DB.prepare(
      "SELECT * FROM clips WHERE user_id = ? ORDER BY created_at DESC LIMIT 30",
    )
      .bind(user.id)
      .all<ClipRow>();
    return render(c, <HomeLoggedIn
        user={user}
        used={await usedBytes(c.env.DB, user.id)}
        quota={userQuota(c.var.config, user)}
        clips={results}
      />);
  });

  app.get("/c/:id", async (c) => {
    const id = c.req.param("id");
    const clip = isClipId(id)
      ? await c.env.DB.prepare("SELECT * FROM clips WHERE id = ? AND status = 'ready'").bind(id).first<ClipRow>()
      : null;
    // Privado de outra pessoa = 404 (não revela que existe).
    if (!clip || (clip.visibility === "private" && c.var.user?.id !== clip.user_id)) {
      return render(c, <Message title={t.notFound} text={t.notFoundText} user={c.var.user} />, 404);
    }
    const owner = await c.env.DB.prepare("SELECT name FROM users WHERE id = ?")
      .bind(clip.user_id)
      .first<Pick<User, "name">>();
    const urls = await clipUrls(c, clip);
    const res = render(c, <ClipPage clip={clip} urls={urls} owner={owner?.name ?? ""} viewer={c.var.user} />);
    if (clip.visibility !== "public") res.headers.set("X-Robots-Tag", "noindex");
    return res;
  });

  app.get("/enviar", (c) => {
    if (!c.var.user) return c.redirect(`/entrar?next=${encodeURIComponent("/enviar")}`);
    return render(c, <UploadPage user={c.var.user} />);
  });

  app.route("/", authRoutes);
  app.route("/", media);
  app.route("/", releases);
  // `accounts` antes de `api`: a API de clipes exige login em tudo, a de contas tem rotas públicas.
  app.route("/api/v1", accounts);
  app.route("/api/v1", api);
  app.route("/admin", admin);

  app.notFound((c) => {
    if (c.req.path.startsWith("/api/")) return apiError(c, 404, "not_found", t.api.notFound);
    return render(c, <Message title={t.notFound} text={t.notFoundText} user={c.var.user} />, 404);
  });

  app.onError((err, c) => {
    console.error(err);
    if (c.req.path.startsWith("/api/")) return apiError(c, 500, "server", t.api.server);
    return render(c, <Message title={t.serverError} text={t.serverError} />, 500);
  });

  return app;
}
