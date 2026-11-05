import { env } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { sha256, b64url } from "../src/lib/crypto";
import { ADMIN_ID, Browser, adminBrowser, call, clock, discordCode, form, makeInvite, userBrowser, vars } from "./helpers";

describe("login web", () => {
  it("admin entra sem convite e vê a página inicial", async () => {
    const b = await adminBrowser();
    const home = await b.fetch("/");
    expect(home.status).toBe(200);
    expect(await home.text()).toContain("Olá, dono");
    const u = await env.DB.prepare("SELECT role FROM users WHERE discord_id = ?").bind(ADMIN_ID).first();
    expect(u?.role).toBe("admin");
  });

  it("servidor novo (sem usuários e sem ADMIN_DISCORD_IDS): o primeiro login vira admin; depois exige convite", async () => {
    vars.ADMIN_DISCORD_IDS = "";
    const first = new Browser();
    const r = await first.login({ id: "400000000000000001", username: "primeiro" });
    expect(r.status).toBe(302);
    expect(r.headers.get("Location")).toBe("/");
    expect(first.session).toBeTruthy();
    expect((await first.fetch("/admin")).status).toBe(200);
    const u = await env.DB.prepare("SELECT role, quota_bytes FROM users WHERE discord_id = ?")
      .bind("400000000000000001")
      .first();
    expect(u).toEqual({ role: "admin", quota_bytes: 0 });

    // O segundo precisa de convite.
    const second = new Browser();
    const r2 = await second.login({ id: "400000000000000002" });
    expect(r2.headers.get("Location")).toBe("/convite");
    expect(second.session).toBeUndefined();

    // O primeiro continua admin ao entrar de novo (o login não rebaixa).
    const again = new Browser();
    expect((await again.login({ id: "400000000000000001" })).status).toBe(302);
    const role = await env.DB.prepare("SELECT role FROM users WHERE discord_id = ?")
      .bind("400000000000000001")
      .first<{ role: string }>();
    expect(role?.role).toBe("admin");
    expect((await again.fetch("/admin")).status).toBe(200);
  });

  it("com ADMIN_DISCORD_IDS definido, o primeiro login de quem não é admin não vira admin", async () => {
    const b = new Browser();
    const r = await b.login({ id: "400000000000000003" });
    expect(r.headers.get("Location")).toBe("/convite");
    expect(b.session).toBeUndefined();
  });

  it("primeiro admin com INVITE_REQUIRED=false também vira admin; o seguinte entra como usuário", async () => {
    vars.ADMIN_DISCORD_IDS = "";
    vars.INVITE_REQUIRED = "false";
    const a = new Browser();
    await a.login({ id: "400000000000000004" });
    const b = new Browser();
    await b.login({ id: "400000000000000005" });
    const rows = await env.DB.prepare("SELECT discord_id, role FROM users ORDER BY created_at, discord_id").all();
    expect(rows.results.map((r) => r.role)).toEqual(["admin", "user"]);
  });

  it("conta nova sem convite vai para /convite e não cria usuário", async () => {
    const b = new Browser();
    const r = await b.login({ id: "200000000000000001" });
    expect(r.status).toBe(302);
    expect(r.headers.get("Location")).toBe("/convite");
    expect(b.session).toBeUndefined();
    const page = await b.fetch("/convite");
    expect(page.status).toBe(200);
    const n = await env.DB.prepare("SELECT COUNT(*) AS n FROM users WHERE discord_id = ?")
      .bind("200000000000000001")
      .first<{ n: number }>();
    expect(n?.n).toBe(0);
  });

  it("código de convite: inválido recusa, válido cria a conta, uso único", async () => {
    const admin = await adminBrowser();
    const code = await makeInvite(admin, 1);

    const b = new Browser();
    await b.login({ id: "200000000000000002", username: "amigo" });
    const bad = await form(b, "/convite", { code: "x".repeat(22) });
    expect(bad.status).toBe(400);
    expect(await bad.text()).toContain("Convite inválido");

    // Colar o link inteiro também vale.
    const ok = await form(b, "/convite", { code: `https://clipe.test/i/${code}` });
    expect(ok.status).toBe(303);
    expect(ok.headers.get("Location")).toBe("/");
    expect(b.session).toBeTruthy();
    expect(await (await b.fetch("/")).text()).toContain("Olá, amigo");

    // Segundo amigo com o mesmo convite (1 uso): recusado.
    const c = new Browser();
    await c.login({ id: "200000000000000003" });
    const again = await form(c, "/convite", { code });
    expect(again.status).toBe(400);
    const inv = await env.DB.prepare("SELECT uses FROM invites").first<{ uses: number }>();
    expect(inv?.uses).toBe(1);
  });

  it("link de convite (/i/<código>) cria a conta direto no login", async () => {
    const admin = await adminBrowser();
    const code = await makeInvite(admin, 2);
    const b = new Browser();
    const r0 = await b.fetch(`/i/${code}`);
    expect(r0.headers.get("Location")).toBe("/entrar");
    const r = await b.login({ id: "200000000000000004" });
    expect(r.headers.get("Location")).toBe("/");
    expect(b.session).toBeTruthy();
  });

  it("convite vencido ou revogado não vale", async () => {
    const admin = await adminBrowser();
    const vencido = await makeInvite(admin, 5, 1);
    clock.now += 2 * 86400_000;
    const b = new Browser();
    await b.fetch(`/i/${vencido}`);
    const r = await b.login({ id: "200000000000000005" });
    expect(r.headers.get("Location")).toBe("/convite?erro=1");

    const revogado = await makeInvite(admin, 5);
    const { id } = (await env.DB.prepare("SELECT id FROM invites ORDER BY created_at DESC LIMIT 1").first<{ id: string }>())!;
    expect((await form(admin, `/admin/convites/${id}/revogar`, {})).status).toBe(303);
    expect((await form(b, "/convite", { code: revogado })).status).toBe(400);
  });

  it("callback exige o state do mesmo navegador", async () => {
    const a = new Browser();
    const r1 = await a.fetch("/entrar");
    const state = new URL(r1.headers.get("Location")!).searchParams.get("state")!;
    // Outro navegador (sem o cookie) usando o state de A: recusado.
    const b = new Browser();
    const r = await b.fetch(`/auth/discord/callback?code=${discordCode({ id: ADMIN_ID })}&state=${state}`);
    expect(r.status).toBe(400);
    expect(b.session).toBeUndefined();
    // E o state só vale uma vez.
    const ok = await a.fetch(`/auth/discord/callback?code=${discordCode({ id: ADMIN_ID })}&state=${state}`);
    expect(ok.status).toBe(302);
    const again = await a.fetch(`/auth/discord/callback?code=${discordCode({ id: ADMIN_ID })}&state=${state}`);
    expect(again.status).toBe(400);
  });

  it("next só aceita caminho relativo (sem open redirect)", async () => {
    const b = new Browser();
    const r = await b.login({ id: ADMIN_ID }, "/entrar?next=//evil.example/x");
    expect(r.headers.get("Location")).toBe("/");
    for (const evil of ["/%09/evil.example", "/%5Cevil.example", "https://evil.example", "/\\evil.example"]) {
      const bx = new Browser();
      const rx = await bx.login({ id: ADMIN_ID }, `/entrar?next=${evil}`);
      expect(rx.headers.get("Location"), evil).toBe("/");
    }
    const b2 = new Browser();
    const r2 = await b2.login({ id: ADMIN_ID }, "/entrar?next=/admin");
    expect(r2.headers.get("Location")).toBe("/admin");
  });

  it("POST com cookie de outra origem (CSRF) é recusado", async () => {
    const admin = await adminBrowser();
    const r = await admin.fetch("/admin/convites", {
      method: "POST",
      headers: { Origin: "https://evil.example", "Content-Type": "application/x-www-form-urlencoded" },
      body: "max_uses=1&days=1",
    });
    expect(r.status).toBe(403);
    const n = await env.DB.prepare("SELECT COUNT(*) AS n FROM invites").first<{ n: number }>();
    expect(n?.n).toBe(0);
  });

  it("sair apaga a sessão", async () => {
    const b = await adminBrowser();
    const token = b.session!;
    expect((await b.fetch("/sair", { method: "POST" })).status).toBe(303);
    expect(b.session).toBeUndefined();
    const s = await env.DB.prepare("SELECT COUNT(*) AS n FROM sessions WHERE token_hash = ?")
      .bind(await sha256(token))
      .first<{ n: number }>();
    expect(s?.n).toBe(0);
  });

  it("não-admin não entra no /admin", async () => {
    const u = await userBrowser("200000000000000006");
    expect((await u.fetch("/admin")).status).toBe(403);
    expect((await form(u, "/admin/convites", { max_uses: "1", days: "1" })).status).toBe(403);
  });

  it("admin desativa usuário: sessões caem e o login é recusado", async () => {
    const admin = await adminBrowser();
    const u = await userBrowser("200000000000000007", admin);
    const { id } = (await env.DB.prepare("SELECT id FROM users WHERE discord_id = ?")
      .bind("200000000000000007")
      .first<{ id: string }>())!;
    expect((await form(admin, `/admin/usuarios/${id}/desativar`, {})).status).toBe(303);
    expect(await (await u.fetch("/")).text()).toContain("Entrar com Discord");
    const again = new Browser();
    expect((await again.login({ id: "200000000000000007" })).status).toBe(403);
  });
});

describe("login do app (loopback + PKCE)", () => {
  async function appLogin(id: string, verifier: string) {
    const challenge = await sha256(verifier);
    const b = new Browser();
    const r = await b.login({ id }, `/auth/app?port=53682&challenge=${challenge}&state=estado-do-app`);
    return { b, r };
  }

  it("devolve código no 127.0.0.1 e troca por token só com o verifier", async () => {
    const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
    const { r, b } = await appLogin(ADMIN_ID, verifier);
    const loc = new URL(r.headers.get("Location")!);
    expect(loc.origin).toBe("http://127.0.0.1:53682");
    expect(loc.pathname).toBe("/callback");
    expect(loc.searchParams.get("state")).toBe("estado-do-app");
    expect(b.session).toBeUndefined(); // login do app não deixa sessão no navegador
    const code = loc.searchParams.get("code")!;

    const wrong = await call("/api/v1/auth/token", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, verifier: b64url(new Uint8Array(32)) }),
    });
    expect(wrong.status).toBe(400);
    // O código é de uso único: a tentativa errada já o consumiu.
    const after = await call("/api/v1/auth/token", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, verifier }),
    });
    expect(after.status).toBe(400);
  });

  it("token Bearer acessa a API; cookie e Bearer não se misturam; logout revoga", async () => {
    const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
    const { r } = await appLogin(ADMIN_ID, verifier);
    const code = new URL(r.headers.get("Location")!).searchParams.get("code")!;
    const res = await call("/api/v1/auth/token", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, verifier }),
    });
    expect(res.status).toBe(200);
    const { token } = (await res.json()) as { token: string };
    const auth = { Authorization: `Bearer ${token}` };

    const me = await call("/api/v1/me", { headers: auth });
    expect(me.status).toBe(200);
    expect(((await me.json()) as { user: { role: string } }).user.role).toBe("admin");

    // Token do app colocado como cookie não vale.
    const asCookie = await call("/api/v1/me", { headers: { Cookie: `__Host-sess=${token}` } });
    expect(asCookie.status).toBe(401);
    // Cookie web usado como Bearer também não.
    const web = await adminBrowser();
    expect((await call("/api/v1/me", { headers: { Authorization: `Bearer ${web.session}` } })).status).toBe(401);

    expect((await call("/api/v1/auth/logout", { method: "POST", headers: auth })).status).toBe(204);
    expect((await call("/api/v1/me", { headers: auth })).status).toBe(401);
  });

  it("parâmetros inválidos são recusados", async () => {
    for (const q of [
      "port=80&challenge=" + "a".repeat(43) + "&state=estado123",
      "port=53682&challenge=curto&state=estado123",
      "port=53682&challenge=" + "a".repeat(43) + "&state=x",
    ]) {
      expect((await call(`/auth/app?${q}`)).status).toBe(400);
    }
  });

  it("convite exigido também no login do app", async () => {
    const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
    const admin = await adminBrowser();
    const invite = await makeInvite(admin);
    const { b, r } = await appLogin("200000000000000010", verifier);
    expect(r.headers.get("Location")).toBe("/convite");
    const ok = await form(b, "/convite", { code: invite });
    expect(ok.status).toBe(303);
    expect(new URL(ok.headers.get("Location")!).origin).toBe("http://127.0.0.1:53682");
  });
});

describe("rate limit", () => {
  it("tentativas de convite por IP são limitadas", async () => {
    const b = new Browser();
    await b.login({ id: "200000000000000020" });
    let last = 0;
    for (let i = 0; i < 11; i++) {
      last = (await form(b, "/convite", { code: "y".repeat(22) })).status;
    }
    expect(last).toBe(429);
  });
});
