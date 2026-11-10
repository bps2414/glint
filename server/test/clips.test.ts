import { env } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { cleanup } from "../src/cron";
import { PART_SIZE } from "../src/env";
import { parseRange } from "../src/lib/range";
import { Browser, JPEG, adminBrowser, call, clock, fakeMp4, userBrowser, vars } from "./helpers";

type Json = Record<string, any>;

const meta = { title: "Ace no Ascent", game: "VALORANT", duration_ms: 30_000, width: 1920, height: 1080 };

async function json(b: Browser, method: string, path: string, body?: unknown) {
  const r = await b.fetch(`/api/v1${path}`, {
    method,
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: r.status, body: (r.status === 204 ? null : await r.json()) as Json };
}

function putPart(b: Browser, id: string, n: number, data: Uint8Array) {
  return b.fetch(`/api/v1/clips/${id}/parts/${n}`, {
    method: "PUT",
    headers: { "Content-Length": String(data.byteLength) },
    body: data,
  });
}

/** Cria, envia tudo e conclui. */
async function publish(b: Browser, size = 4096, extra: Json = {}) {
  const data = fakeMp4(size);
  const created = await json(b, "POST", "/clips", { ...meta, size_bytes: size, ...extra });
  expect(created.status).toBe(201);
  const { clip, upload } = created.body;
  for (let n = 1; n <= upload.part_count; n++) {
    const part = data.subarray((n - 1) * upload.part_size, n * upload.part_size);
    expect((await putPart(b, clip.id, n, part)).status).toBe(200);
  }
  const thumb = await b.fetch(`/api/v1/clips/${clip.id}/thumb`, {
    method: "PUT",
    headers: { "Content-Type": "image/jpeg", "Content-Length": String(JPEG.byteLength) },
    body: JPEG,
  });
  expect(thumb.status).toBe(204);
  const done = await json(b, "POST", `/clips/${clip.id}/complete`);
  expect(done.status).toBe(200);
  return { clip: done.body.clip as Json, data };
}

describe("upload e página do clipe", () => {
  it("fluxo completo: página com embed do Discord e vídeo com Range", async () => {
    const b = await adminBrowser();
    const { clip, data } = await publish(b, 5000);
    expect(clip.status).toBe("ready");
    expect(clip.visibility).toBe("unlisted");
    expect(clip.url).toBe(`https://clipe.test/c/${clip.id}`);
    expect(clip.id).toMatch(/^[A-Za-z0-9_-]{22}$/);

    // Página pública (sem login) com as meta tags do embed.
    const page = await call(`/c/${clip.id}`);
    expect(page.status).toBe(200);
    const html = await page.text();
    expect(html).toContain(`<meta property="og:video" content="https://clipe.test/v/${clip.id}.mp4"/>`);
    expect(html).toContain(`<meta property="og:video:type" content="video/mp4"/>`);
    expect(html).toContain(`<meta property="og:image" content="https://clipe.test/t/${clip.id}.jpg"/>`);
    expect(html).toContain("Ace no Ascent");
    expect(page.headers.get("Content-Security-Policy")).toContain("default-src 'none'");
    expect(page.headers.get("X-Robots-Tag")).toBe("noindex");

    const full = await call(`/v/${clip.id}.mp4`);
    expect(full.status).toBe(200);
    expect(full.headers.get("Content-Type")).toBe("video/mp4");
    expect(full.headers.get("Accept-Ranges")).toBe("bytes");
    expect(new Uint8Array(await full.arrayBuffer())).toEqual(data);

    const part = await call(`/v/${clip.id}.mp4`, { headers: { Range: "bytes=100-199" } });
    expect(part.status).toBe(206);
    expect(part.headers.get("Content-Range")).toBe("bytes 100-199/5000");
    expect(new Uint8Array(await part.arrayBuffer())).toEqual(data.subarray(100, 200));

    const tail = await call(`/v/${clip.id}.mp4`, { headers: { Range: "bytes=-10" } });
    expect(tail.headers.get("Content-Range")).toBe("bytes 4990-4999/5000");

    const bad = await call(`/v/${clip.id}.mp4`, { headers: { Range: "bytes=6000-" } });
    expect(bad.status).toBe(416);
    expect(bad.headers.get("Content-Range")).toBe("bytes */5000");

    const head = await call(`/v/${clip.id}.mp4`, { method: "HEAD" });
    expect(head.status).toBe(200);
    expect(head.headers.get("Content-Length")).toBe("5000");

    const thumb = await call(`/t/${clip.id}.jpg`);
    expect(thumb.status).toBe(200);
    expect(new Uint8Array(await thumb.arrayBuffer())).toEqual(JPEG);
  });

  it("upload em várias partes pode ser retomado", async () => {
    const b = await adminBrowser();
    const size = PART_SIZE * 2 + 123;
    const data = fakeMp4(size);
    const { body } = await json(b, "POST", "/clips", { ...meta, size_bytes: size });
    const id = body.clip.id;
    expect(body.upload).toEqual({ part_size: PART_SIZE, part_count: 3, parts_done: [] });

    // Parte com tamanho errado é recusada.
    expect((await putPart(b, id, 1, data.subarray(0, 100))).status).toBe(400);
    // Fora de ordem (3, depois 1); "cai a conexão" antes da 2.
    expect((await putPart(b, id, 3, data.subarray(2 * PART_SIZE))).status).toBe(200);
    expect((await putPart(b, id, 1, data.subarray(0, PART_SIZE))).status).toBe(200);
    const early = await json(b, "POST", `/clips/${id}/complete`);
    expect(early.status).toBe(409);
    expect(early.body.error.code).toBe("missing_parts");

    // O app pergunta o que falta e continua.
    const state = await json(b, "GET", `/clips/${id}`);
    expect(state.body.upload.parts_done).toEqual([1, 3]);
    expect((await putPart(b, id, 2, data.subarray(PART_SIZE, 2 * PART_SIZE))).status).toBe(200);
    const done = await json(b, "POST", `/clips/${id}/complete`);
    expect(done.status).toBe(200);
    const video = await call(`/v/${id}.mp4`, { headers: { Range: `bytes=${PART_SIZE - 5}-${PART_SIZE + 4}` } });
    expect(new Uint8Array(await video.arrayBuffer())).toEqual(data.subarray(PART_SIZE - 5, PART_SIZE + 5));
    // Concluir de novo é idempotente.
    expect((await json(b, "POST", `/clips/${id}/complete`)).status).toBe(200);
  });

  it("arquivo que não é MP4 é recusado e apagado", async () => {
    const b = await adminBrowser();
    const { body } = await json(b, "POST", "/clips", { ...meta, size_bytes: 100 });
    expect((await putPart(b, body.clip.id, 1, new Uint8Array(100))).status).toBe(200);
    const done = await json(b, "POST", `/clips/${body.clip.id}/complete`);
    expect(done.status).toBe(422);
    expect((await json(b, "GET", `/clips/${body.clip.id}`)).status).toBe(404);
    expect((await env.BUCKET.list()).objects).toHaveLength(0);
  });

  it("validação: campos, tamanho máximo e miniatura", async () => {
    const b = await adminBrowser();
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: -1 })).status).toBe(400);
    expect((await json(b, "POST", "/clips", { ...meta, title: "a\nb", size_bytes: 10 })).status).toBe(400);
    expect((await json(b, "POST", "/clips", { ...meta, visibility: "secreto", size_bytes: 10 })).status).toBe(400);
    const big = await json(b, "POST", "/clips", { ...meta, size_bytes: 501 * 1024 * 1024 });
    expect(big.status).toBe(413);

    const { body } = await json(b, "POST", "/clips", { ...meta, size_bytes: 10 });
    const notJpeg = await b.fetch(`/api/v1/clips/${body.clip.id}/thumb`, {
      method: "PUT",
      headers: { "Content-Length": "4" },
      body: new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
    });
    expect(notJpeg.status).toBe(400);
  });

  it("sem USER_QUOTA_GB não há cota por usuário (mesmo com quota_bytes pequeno no banco)", async () => {
    const admin = await adminBrowser();
    const u = await userBrowser("300000000000000002", admin);
    await env.DB.prepare("UPDATE users SET quota_bytes = 100 WHERE discord_id = ?").bind("300000000000000002").run();
    await publish(u, 6000);
    expect((await json(u, "POST", "/clips", { ...meta, size_bytes: 5000 })).status).toBe(201);
    expect((await json(u, "GET", "/me")).body.quota.quota_bytes).toBe(0);
    expect(await (await u.fetch("/")).text()).not.toContain("<progress");
  });

  it("cota por usuário (USER_QUOTA_GB): reserva na criação e recusa quando passa do limite", async () => {
    vars.USER_QUOTA_GB = "0.00001"; // ~10737 bytes de padrão
    const admin = await adminBrowser();
    const u = await userBrowser("300000000000000001", admin);
    await env.DB.prepare("UPDATE users SET quota_bytes = 10000 WHERE discord_id = ?").bind("300000000000000001").run();
    await publish(u, 6000);
    // 6000 + 10 (miniatura) usados; mais 5000 passa de 10000.
    const over = await json(u, "POST", "/clips", { ...meta, size_bytes: 5000 });
    expect(over.status).toBe(409);
    expect(over.body.error.code).toBe("quota_exceeded");
    expect(over.body.error.used_bytes).toBe(6010);
    expect(over.body.error.quota_bytes).toBe(10000);
    expect(over.body.error.needed_bytes).toBe(1010);
    const me = await json(u, "GET", "/me");
    expect(me.body.quota).toEqual({ used_bytes: 6010, quota_bytes: 10000 });
    // Apagar libera.
    const list = await json(u, "GET", "/clips");
    expect((await json(u, "DELETE", `/clips/${list.body.clips[0].id}`)).status).toBe(204);
    expect((await json(u, "POST", "/clips", { ...meta, size_bytes: 5000 })).status).toBe(201);
  });

  it("cota ligada: usuário sem cota própria (0) usa o padrão do servidor", async () => {
    vars.USER_QUOTA_GB = "0.00001";
    const admin = await adminBrowser();
    const u = await userBrowser("300000000000000003", admin);
    await env.DB.prepare("UPDATE users SET quota_bytes = 0 WHERE discord_id = ?").bind("300000000000000003").run();
    expect((await json(u, "GET", "/me")).body.quota.quota_bytes).toBe(Math.round(0.00001 * 1024 ** 3));
    expect((await json(u, "POST", "/clips", { ...meta, size_bytes: 20_000 })).status).toBe(409);
    expect((await json(u, "POST", "/clips", { ...meta, size_bytes: 10_000 })).status).toBe(201);
  });

  it("trava global (STORAGE_LIMIT_GB): recusa com 507 storage_full, vale para todos, apagar libera", async () => {
    vars.STORAGE_LIMIT_GB = "0.00001"; // 10737 bytes no servidor todo
    const admin = await adminBrowser();
    const a = await userBrowser("300000000000000004", admin);
    const b = await userBrowser("300000000000000005", admin);
    await publish(a, 6000); // 6010 com a miniatura
    const full = await json(b, "POST", "/clips", { ...meta, size_bytes: 5000 });
    expect(full.status).toBe(507);
    expect(full.body.error.code).toBe("storage_full");
    expect(full.body.error.used_bytes).toBe(6010);
    expect(full.body.error.limit_bytes).toBe(Math.round(0.00001 * 1024 ** 3));
    // O que cabe entra (conta a soma do servidor, não cada usuário).
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 4000 })).status).toBe(201);
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 4000 })).status).toBe(507);
    const list = await json(a, "GET", "/clips");
    expect((await json(a, "DELETE", `/clips/${list.body.clips[0].id}`)).status).toBe(204);
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 5000 })).status).toBe(201);
  });

  it("sem espaço: sugere os mais antigos sem favoritos e não apaga nada", async () => {
    vars.STORAGE_LIMIT_GB = "0.00001"; // 10737 bytes
    const b = await adminBrowser();
    const c1 = (await publish(b, 3000)).clip;
    clock.now += 1000;
    const c2 = (await publish(b, 3000, { favorite: true })).clip;
    clock.now += 1000;
    const c3 = (await publish(b, 3000)).clip;
    expect(c2.favorite).toBe(true);
    expect(c1.favorite).toBe(false);

    const full = await json(b, "POST", "/clips", { ...meta, size_bytes: 5000 });
    expect(full.status).toBe(507);
    expect(full.body.error.needed_bytes).toBe(3 * 3010 + 5000 - 10737);

    const s = await json(b, "GET", `/clips/cleanup?bytes=${full.body.error.needed_bytes}`);
    expect(s.status).toBe(200);
    expect(s.body.clips.map((c: Json) => c.id)).toEqual([c1.id, c3.id]);
    expect(s.body.freed_bytes).toBe(6020);
    expect(s.body.enough).toBe(true);

    // Favoritar pelo PATCH tira da lista; aí não dá mais.
    expect((await json(b, "PATCH", `/clips/${c1.id}`, { favorite: true })).body.clip.favorite).toBe(true);
    const s2 = await json(b, "GET", `/clips/cleanup?bytes=${full.body.error.needed_bytes}`);
    expect(s2.body.clips.map((c: Json) => c.id)).toEqual([c3.id]);
    expect(s2.body.enough).toBe(false);

    expect((await json(b, "GET", "/clips")).body.clips).toHaveLength(3);
    expect((await json(b, "GET", "/clips/cleanup?bytes=0")).status).toBe(400);
  });

  it("trava global: padrão de 9,5 GB aceita clipes normais e recusa o que passa", async () => {
    const b = await adminBrowser();
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 400 * 1024 ** 2 })).status).toBe(201);
    vars.MAX_CLIP_MB = "20000";
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 10 * 1024 ** 3 })).status).toBe(507);
  });

  it("upload pela metade expirado libera espaço da trava global depois da faxina", async () => {
    vars.STORAGE_LIMIT_GB = "0.00001";
    const b = await adminBrowser();
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 8000 })).status).toBe(201);
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 8000 })).status).toBe(507);
    await cleanup(env, clock.now + 25 * 3600_000);
    expect((await json(b, "POST", "/clips", { ...meta, size_bytes: 8000 })).status).toBe(201);
  });

  it("clipe de outra pessoa: invisível na API", async () => {
    const admin = await adminBrowser();
    const { clip } = await publish(admin);
    const u = await userBrowser("300000000000000002", admin);
    expect((await json(u, "GET", `/clips/${clip.id}`)).status).toBe(404);
    expect((await json(u, "PATCH", `/clips/${clip.id}`, { title: "meu" })).status).toBe(404);
    expect((await json(u, "DELETE", `/clips/${clip.id}`)).status).toBe(404);
    expect((await putPart(u, clip.id, 1, new Uint8Array(10))).status).toBe(404);
    expect((await json(u, "GET", "/clips")).body.clips).toEqual([]);
    expect((await call("/api/v1/clips")).status).toBe(401);
  });

  it("privado: página só para o dono, mídia só com URL assinada válida", async () => {
    const admin = await adminBrowser();
    const { clip } = await publish(admin, 4096, { visibility: "private" });
    expect(clip.video_url).toMatch(/\/v\/[\w-]{22}\.mp4\?exp=\d+&sig=[\w-]{43}$/);

    expect((await call(`/c/${clip.id}`)).status).toBe(404);
    const u = await userBrowser("300000000000000003", admin);
    expect((await u.fetch(`/c/${clip.id}`)).status).toBe(404);
    const own = await admin.fetch(`/c/${clip.id}`);
    expect(own.status).toBe(200);
    const html = await own.text();
    expect(html).not.toContain("og:video");

    expect((await call(`/v/${clip.id}.mp4`)).status).toBe(404);
    const signed = new URL(clip.video_url);
    expect((await call(signed.pathname + signed.search)).status).toBe(200);
    // Assinatura de outro tipo de mídia (miniatura) ou adulterada não vale.
    const forged = signed.search.replace(/sig=([\w-])/, (_, ch) => `sig=${ch === "A" ? "B" : "A"}`);
    expect((await call(signed.pathname + forged)).status).toBe(404);
    expect((await call(`/t/${clip.id}.jpg${signed.search}`)).status).toBe(404);
    // Vence em 1 h.
    clock.now += 3601_000;
    expect((await call(signed.pathname + signed.search)).status).toBe(404);

    // Tornar "com link" libera a página e a mídia.
    expect((await json(admin, "PATCH", `/clips/${clip.id}`, { visibility: "unlisted" })).status).toBe(200);
    expect((await call(`/v/${clip.id}.mp4`)).status).toBe(200);
  });

  it("apagar remove do R2 e o link para de funcionar", async () => {
    const b = await adminBrowser();
    const { clip } = await publish(b);
    expect((await json(b, "DELETE", `/clips/${clip.id}`)).status).toBe(204);
    expect((await call(`/c/${clip.id}`)).status).toBe(404);
    expect((await call(`/v/${clip.id}.mp4`)).status).toBe(404);
    expect((await env.BUCKET.list()).objects).toHaveLength(0);
  });

  it("faxina apaga uploads abandonados depois de 24 h", async () => {
    const b = await adminBrowser();
    const { body } = await json(b, "POST", "/clips", { ...meta, size_bytes: 100 });
    await publish(b);
    expect((await cleanup(env, clock.now)).abandonedUploads).toBe(0);
    const r = await cleanup(env, clock.now + 25 * 3600_000);
    expect(r.abandonedUploads).toBe(1);
    expect((await json(b, "GET", `/clips/${body.clip.id}`)).status).toBe(404);
    expect((await json(b, "GET", "/clips")).body.clips).toHaveLength(1);
  });

  it("página de envio exige login", async () => {
    const r = await call("/enviar");
    expect(r.status).toBe(302);
    expect(r.headers.get("Location")).toBe("/entrar?next=%2Fenviar");
    const b = await adminBrowser();
    expect((await b.fetch("/enviar")).status).toBe(200);
  });
});

describe("parseRange", () => {
  it("formas aceitas e recusadas", () => {
    expect(parseRange(null, 100)).toBeNull();
    expect(parseRange("bytes=0-", 100)).toEqual({ offset: 0, length: 100 });
    expect(parseRange("bytes=10-20", 100)).toEqual({ offset: 10, length: 11 });
    expect(parseRange("bytes=90-500", 100)).toEqual({ offset: 90, length: 10 });
    expect(parseRange("bytes=-500", 100)).toEqual({ offset: 0, length: 100 });
    expect(parseRange("bytes=0-1,5-6", 100)).toBeNull();
    expect(parseRange("items=0-1", 100)).toBeNull();
    expect(parseRange("bytes=100-", 100)).toBe("unsatisfiable");
    expect(parseRange("bytes=20-10", 100)).toBe("unsatisfiable");
    expect(parseRange("bytes=-0", 100)).toBe("unsatisfiable");
  });
});
