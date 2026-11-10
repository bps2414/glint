import { applyD1Migrations, createExecutionContext, env as rawEnv, waitOnExecutionContext } from "cloudflare:test";
import { beforeAll, describe, expect, it } from "vitest";
import { type Env, MAX_LOGS, type OpenIssue, type Report, createHandler, issueContent } from "../index";

const env = rawEnv as unknown as Env & { TEST_MIGRATIONS: Parameters<typeof applyD1Migrations>[1] };
beforeAll(() => applyD1Migrations(env.DB, env.TEST_MIGRATIONS));

let ipSeq = 0;
const issues: Report[] = [];
const okIssue: OpenIssue = async (_e, r) => {
  issues.push(r);
  return `https://github.com/bps2414/glint/issues/${issues.length}`;
};

async function send(body: unknown, opts: { ip?: string; issue?: OpenIssue; client?: boolean } = {}) {
  const ctx = createExecutionContext();
  const headers: Record<string, string> = { "content-type": "application/json", "cf-connecting-ip": opts.ip ?? `10.0.0.${++ipSeq}` };
  if (opts.client !== false) headers["x-glint-client"] = "1.1.0";
  const req = new Request("https://feedback.test/v1/report", { method: "POST", headers, body: typeof body === "string" ? body : JSON.stringify(body) });
  const res = await createHandler(opts.issue ?? okIssue)(req, env, ctx);
  await waitOnExecutionContext(ctx);
  return res;
}

const row = (id: string) => env.DB.prepare("SELECT * FROM reports WHERE id = ?").bind(id).first<Record<string, unknown>>();

describe("POST /v1/report", () => {
  it("salva o bug, o log no R2 e a issue sem logs nem contato", async () => {
    const res = await send({ kind: "bug", message: "Travou ao salvar\nsegunda linha", contact: "eu@x.com", app_version: "1.1.0", gpu: "RTX", logs: "linha de log" });
    expect(res.status).toBe(200);
    const { id } = (await res.json()) as { id: string };
    const r = await row(id);
    expect(r).toMatchObject({ kind: "bug", contact: "eu@x.com", has_logs: 1 });
    expect(r?.issue_url).toMatch(/issues\/\d+$/);
    expect(await (await env.BUCKET.get(`reports/${id}.log`))?.text()).toBe("linha de log");
    const issue = issues.at(-1)!;
    expect(JSON.stringify(issue)).not.toContain("eu@x.com");
    expect(JSON.stringify(issue)).not.toContain("linha de log");
    expect(issue.specs).toEqual({ app_version: "1.1.0", gpu: "RTX" });
  });

  it("falha do GitHub não perde o report", async () => {
    const res = await send({ kind: "feedback", message: "gostei" }, { issue: async () => { throw new Error("500"); } });
    expect(res.status).toBe(200);
    const { id } = (await res.json()) as { id: string };
    expect(await row(id)).toMatchObject({ kind: "feedback", issue_url: null, has_logs: 0 });
  });

  it("valida campos e limites", async () => {
    expect((await send({ kind: "x", message: "a" })).status).toBe(400);
    expect((await send({ kind: "bug", message: "  " })).status).toBe(400);
    expect((await send("{nao json")).status).toBe(400);
    expect((await send({ kind: "bug", message: "a".repeat(5001) })).status).toBe(413);
    expect((await send({ kind: "bug", message: "a", logs: "x".repeat(MAX_LOGS + 1) })).status).toBe(413);
    expect((await send({ kind: "bug", message: "a" }, { client: false })).status).toBe(400);
  });

  it("rate limit por IP", async () => {
    const codes = [];
    for (let i = 0; i < 5; i++) codes.push((await send({ kind: "feedback", message: `m${i}` }, { ip: "9.9.9.9" })).status);
    expect(codes).toContain(429);
    expect(codes.slice(0, 3)).toEqual([200, 200, 200]);
  });
});

describe("GET /v1/reports/:id/logs", () => {
  it("só com o token de admin", async () => {
    const { id } = (await (await send({ kind: "bug", message: "a", logs: "segredo" })).json()) as { id: string };
    const get = (auth?: string) =>
      createHandler(okIssue)(new Request(`https://feedback.test/v1/reports/${id}/logs`, { headers: auth ? { authorization: auth } : {} }), env, createExecutionContext());
    expect((await get()).status).toBe(401);
    expect((await get("Bearer errado")).status).toBe(401);
    const ok = await get("Bearer admin-de-teste");
    expect(ok.status).toBe(200);
    expect(await ok.text()).toBe("segredo");
  });
});

describe("issueContent", () => {
  it("neutraliza menções e corta o título", () => {
    const c = issueContent({ id: "abc", kind: "bug", message: `@alguem ${"x".repeat(100)}`, specs: {} });
    expect(c.title.startsWith("[bug] @​alguem")).toBe(true);
    expect(c.title.length).toBeLessThanOrEqual(6 + 80);
    expect(c.body).toContain("> @​alguem");
  });
});
