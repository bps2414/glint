// Recebe feedback e bug reports do app: guarda no D1 (texto) e no R2 (logs), privados, e abre uma
// issue pública no GitHub só com a mensagem e as specs (sem logs e sem contato).

export interface Env {
  DB: D1Database;
  BUCKET: R2Bucket;
  LIMITER: { limit(o: { key: string }): Promise<{ success: boolean }> };
  GITHUB_REPO: string;
  GITHUB_TOKEN?: string;
  ADMIN_TOKEN?: string;
}

export interface Report {
  id: string;
  kind: "bug" | "feedback";
  message: string;
  specs: Record<string, string>;
}

/** Cria a issue e devolve a URL. Injetável nos testes. */
export type OpenIssue = (env: Env, r: Report) => Promise<string>;

export const MAX_MESSAGE = 5000;
export const MAX_CONTACT = 200;
export const MAX_LOGS = 512 * 1024;
const MAX_SPEC = 200;
const MAX_BODY = MAX_LOGS + 64 * 1024;
const SPEC_FIELDS = ["app_version", "os", "gpu", "capture_backend", "locale"] as const;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
const fail = (status: number, error: string) => json({ error }, status);

export function createHandler(openIssue: OpenIssue = githubIssue) {
  return async (req: Request, env: Env, ctx: ExecutionContext): Promise<Response> => {
    const url = new URL(req.url);
    if (url.pathname === "/v1/report") {
      if (req.method !== "POST") return fail(405, "method");
      return report(req, env, ctx, openIssue);
    }
    const logs = url.pathname.match(/^\/v1\/reports\/([0-9a-f-]{36})\/logs$/);
    if (logs && req.method === "GET") return adminLogs(req, env, logs[1]!);
    return fail(404, "not_found");
  };
}

async function report(req: Request, env: Env, ctx: ExecutionContext, openIssue: OpenIssue): Promise<Response> {
  if (!req.headers.get("x-glint-client")) return fail(400, "client");
  if (Number(req.headers.get("content-length") ?? 0) > MAX_BODY) return fail(413, "too_large");
  const ip = req.headers.get("cf-connecting-ip") ?? "unknown";
  if (!(await env.LIMITER.limit({ key: ip })).success) return fail(429, "rate_limited");

  const text = await req.text();
  if (text.length > MAX_BODY) return fail(413, "too_large");
  let body: Record<string, unknown>;
  try {
    body = JSON.parse(text);
  } catch {
    return fail(400, "json");
  }
  if (typeof body !== "object" || body === null) return fail(400, "json");

  const kind = body.kind;
  if (kind !== "bug" && kind !== "feedback") return fail(400, "kind");
  const message = typeof body.message === "string" ? body.message.trim() : "";
  if (!message) return fail(400, "message");
  if (message.length > MAX_MESSAGE) return fail(413, "message_too_long");
  const contact = typeof body.contact === "string" ? body.contact.trim() : "";
  if (contact.length > MAX_CONTACT) return fail(413, "contact_too_long");
  const logs = typeof body.logs === "string" ? body.logs : "";
  if (logs.length > MAX_LOGS) return fail(413, "logs_too_large");
  const specs: Record<string, string> = {};
  for (const f of SPEC_FIELDS) {
    const v = body[f];
    if (typeof v === "string" && v.trim()) specs[f] = v.trim().slice(0, MAX_SPEC);
  }

  const id = crypto.randomUUID();
  if (logs) await env.BUCKET.put(`reports/${id}.log`, logs, { httpMetadata: { contentType: "text/plain; charset=utf-8" } });
  await env.DB.prepare(
    "INSERT INTO reports (id, kind, message, contact, specs, has_logs, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
  )
    .bind(id, kind, message, contact || null, JSON.stringify(specs), logs ? 1 : 0, Date.now())
    .run();

  // A issue é extra: se falhar, o report já está salvo.
  ctx.waitUntil(
    openIssue(env, { id, kind, message, specs })
      .then((issue) => env.DB.prepare("UPDATE reports SET issue_url = ? WHERE id = ?").bind(issue, id).run())
      .catch((e) => console.error("issue", id, String(e))),
  );
  return json({ id });
}

async function adminLogs(req: Request, env: Env, id: string): Promise<Response> {
  const auth = req.headers.get("authorization") ?? "";
  if (!env.ADMIN_TOKEN || !(await sameText(auth, `Bearer ${env.ADMIN_TOKEN}`))) return fail(401, "unauthorized");
  const obj = await env.BUCKET.get(`reports/${id}.log`);
  if (!obj) return fail(404, "not_found");
  return new Response(obj.body, { headers: { "content-type": "text/plain; charset=utf-8" } });
}

async function sameText(a: string, b: string): Promise<boolean> {
  const enc = new TextEncoder();
  const [x, y] = await Promise.all([a, b].map((s) => crypto.subtle.digest("SHA-256", enc.encode(s))));
  return crypto.subtle.timingSafeEqual(x!, y!);
}

/** Título e corpo da issue; `@` neutralizado pra não marcar ninguém. */
export function issueContent(r: Report): { title: string; body: string } {
  const safe = (s: string) => s.replace(/@/g, "@​");
  const first = safe(r.message.split("\n")[0]!.trim());
  const title = `[${r.kind}] ${first.length > 80 ? `${first.slice(0, 77)}...` : first}`;
  const quoted = safe(r.message)
    .split("\n")
    .map((l) => `> ${l}`)
    .join("\n");
  const specs = Object.entries(r.specs)
    .map(([k, v]) => `| ${k} | ${safe(v).replace(/\|/g, "\\|")} |`)
    .join("\n");
  const table = specs ? `\n\n| | |\n|---|---|\n${specs}` : "";
  return { title, body: `${quoted}${table}\n\nReport \`${r.id}\` (sent from the app)` };
}

const githubIssue: OpenIssue = async (env, r) => {
  if (!env.GITHUB_TOKEN) throw new Error("GITHUB_TOKEN ausente");
  const { title, body } = issueContent(r);
  const res = await fetch(`https://api.github.com/repos/${env.GITHUB_REPO}/issues`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${env.GITHUB_TOKEN}`,
      accept: "application/vnd.github+json",
      "user-agent": "glint-feedback",
      "x-github-api-version": "2022-11-28",
    },
    body: JSON.stringify({ title, body, labels: [r.kind, "from-app"] }),
  });
  if (!res.ok) throw new Error(`GitHub ${res.status}`);
  return ((await res.json()) as { html_url: string }).html_url;
};

const handler = createHandler();
export default { fetch: handler } satisfies ExportedHandler<Env>;
