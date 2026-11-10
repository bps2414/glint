// App de teste: Discord falso, relógio controlável, "navegador" com cookies.
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { createApp } from "../src/app";
import type { DiscordApi, DiscordProfile } from "../src/auth/discord";

export const ORIGIN = "https://clipe.test";
export const ADMIN_ID = "100000000000000001";

export const clock = { now: Date.UTC(2026, 10, 4, 12) };

/** Perfil devolvido para cada `code` que o "Discord" emitir. */
const codes = new Map<string, DiscordProfile>();
let nextCode = 1;

export function discordCode(profile: Partial<DiscordProfile> & { id: string }): string {
  const code = `code-${nextCode++}`;
  codes.set(code, { username: `user${profile.id.slice(-4)}`, global_name: null, avatar: null, ...profile });
  return code;
}

const fakeDiscord: DiscordApi = {
  authorizeUrl: ({ state, redirectUri }) =>
    `https://discord.test/authorize?state=${encodeURIComponent(state)}&redirect_uri=${encodeURIComponent(redirectUri)}`,
  async profile({ code, clientSecret }) {
    if (clientSecret !== "segredo-de-teste") throw new Error("segredo errado");
    const p = codes.get(code);
    if (!p) throw new Error("code desconhecido");
    codes.delete(code);
    return p;
  },
};

/** Variáveis do Worker trocadas só neste teste (setup.ts limpa a cada teste). */
export const vars: Record<string, string> = {};

export const app = createApp({ discord: fakeDiscord, now: () => clock.now });

export async function call(path: string, init: RequestInit = {}): Promise<Response> {
  const ctx = createExecutionContext();
  const res = await app.request(new Request(new URL(path, ORIGIN), init), undefined, { ...env, ...vars }, ctx);
  await waitOnExecutionContext(ctx);
  return res;
}

/** Cliente com pote de cookies e Origin certo nos POST (como um navegador). */
export class Browser {
  cookies = new Map<string, string>();

  async fetch(path: string, init: RequestInit = {}): Promise<Response> {
    const headers = new Headers(init.headers);
    if (this.cookies.size) {
      headers.set("Cookie", [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; "));
    }
    const method = (init.method ?? "GET").toUpperCase();
    if (method !== "GET" && method !== "HEAD" && !headers.has("Origin")) headers.set("Origin", ORIGIN);
    const res = await call(path, { ...init, headers, redirect: "manual" });
    for (const sc of res.headers.getSetCookie()) {
      const [pair, ...attrs] = sc.split(";");
      const i = pair!.indexOf("=");
      const name = pair!.slice(0, i).trim();
      const value = pair!.slice(i + 1).trim();
      const expired = attrs.some((a) => /max-age=0\b/i.test(a.trim()) || /expires=Thu, 01 Jan 1970/i.test(a));
      if (!value || expired) this.cookies.delete(name);
      else this.cookies.set(name, value);
    }
    return res;
  }

  /** Login completo pelo navegador: /entrar → "Discord" → callback. Devolve o último redirect. */
  async login(profile: Partial<DiscordProfile> & { id: string }, start = "/entrar"): Promise<Response> {
    const r1 = await this.fetch(start);
    const authorize = new URL(r1.headers.get("Location")!);
    const state = authorize.searchParams.get("state")!;
    const code = discordCode(profile);
    return this.fetch(`/auth/discord/callback?code=${code}&state=${encodeURIComponent(state)}`);
  }

  get session() {
    return this.cookies.get("__Host-sess");
  }
}

export async function form(b: Browser, path: string, data: Record<string, string>) {
  return b.fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams(data),
  });
}

/** Admin logado (ID em ADMIN_DISCORD_IDS: entra sem convite). */
export async function adminBrowser(): Promise<Browser> {
  const b = new Browser();
  const r = await b.login({ id: ADMIN_ID, username: "dono" });
  if (r.status !== 302 || !b.session) throw new Error(`login do admin falhou: ${r.status}`);
  return b;
}

/** Gera um convite pelo painel e devolve o código. */
export async function makeInvite(admin: Browser, maxUses = 1, days = 7): Promise<string> {
  const r = await form(admin, "/admin/convites", { note: "teste", max_uses: String(maxUses), days: String(days) });
  const html = await r.text();
  const m = /\/i\/([A-Za-z0-9_-]{22})/.exec(html);
  if (!m) throw new Error(`convite não apareceu: ${r.status}`);
  return m[1]!;
}

/** Usuário comum com conta criada por convite. */
export async function userBrowser(id: string, admin?: Browser): Promise<Browser> {
  const a = admin ?? (await adminBrowser());
  const code = await makeInvite(a);
  const b = new Browser();
  await b.fetch(`/i/${code}/discord`);
  const r = await b.login({ id, username: `u${id.slice(-3)}` });
  if (r.status !== 302 || !b.session) throw new Error(`cadastro falhou: ${r.status} ${r.headers.get("Location")}`);
  return b;
}

/** MP4 mínimo: cabeçalho "ftyp" + enchimento até `size` bytes. */
export function fakeMp4(size: number): Uint8Array {
  const b = new Uint8Array(size);
  b.set([0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d]); // ....ftypisom
  for (let i = 12; i < size; i++) b[i] = i % 251;
  return b;
}

export const JPEG = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4, 0xff, 0xd9]);
