// Vídeo e miniatura servidos do R2 pelo Worker (com Range, para o player e o Discord).
import { Hono, type Context } from "hono";
import type { AppEnv } from "./env";
import { isClipId, thumbKey, verifyMedia, videoKey, type ClipRow } from "./lib/clips";
import { parseRange } from "./lib/range";

export const media = new Hono<AppEnv>();

type Ctx = Context<AppEnv>;

/** Clipe pronto que este pedido pode ver (privado: só com URL assinada válida). */
async function visibleClip(c: Ctx, id: string, kind: "v" | "t"): Promise<ClipRow | null> {
  if (!isClipId(id)) return null;
  const clip = await c.env.DB.prepare("SELECT * FROM clips WHERE id = ? AND status = 'ready'")
    .bind(id)
    .first<ClipRow>();
  if (!clip) return null;
  if (clip.visibility === "private") {
    const ok = await verifyMedia(c.env, kind, id, c.req.query("exp"), c.req.query("sig"), c.var.deps.now());
    if (!ok) return null;
  }
  return clip;
}

function cacheHeaders(clip: ClipRow, h: Headers) {
  if (clip.visibility === "private") {
    h.set("Cache-Control", "private, no-store");
  } else {
    h.set("Cache-Control", "public, max-age=3600");
  }
  if (clip.visibility !== "public") h.set("X-Robots-Tag", "noindex");
}

const notFound = (c: Ctx) => c.text("Não encontrado", 404);

async function serveVideo(c: Ctx, headOnly: boolean) {
  const id = c.req.param("file")?.replace(/\.mp4$/, "") ?? "";
  const clip = await visibleClip(c, id, "v");
  if (!clip) return notFound(c);

  const size = clip.size_bytes;
  const h = new Headers({ "Content-Type": "video/mp4", "Accept-Ranges": "bytes" });
  cacheHeaders(clip, h);
  if (c.req.query("download") === "1") {
    h.set("Content-Disposition", `attachment; filename="clipe-${clip.id}.mp4"`);
  }

  const range = parseRange(c.req.header("Range"), size);
  if (range === "unsatisfiable") {
    h.set("Content-Range", `bytes */${size}`);
    return new Response(null, { status: 416, headers: h });
  }

  if (headOnly) {
    const obj = await c.env.BUCKET.head(videoKey(clip.id));
    if (!obj) return notFound(c);
    h.set("ETag", obj.httpEtag);
    h.set("Content-Length", String(size));
    return new Response(null, { status: 200, headers: h });
  }

  const obj = await c.env.BUCKET.get(videoKey(clip.id), range ? { range } : undefined);
  if (!obj) return notFound(c);
  h.set("ETag", obj.httpEtag);
  if (range) {
    h.set("Content-Range", `bytes ${range.offset}-${range.offset + range.length - 1}/${size}`);
    h.set("Content-Length", String(range.length));
    return new Response(obj.body, { status: 206, headers: h });
  }
  h.set("Content-Length", String(size));
  return new Response(obj.body, { status: 200, headers: h });
}

media.on(["GET", "HEAD"], "/v/:file{[A-Za-z0-9_-]{22}\\.mp4}", (c) => serveVideo(c, c.req.method === "HEAD"));

media.get("/t/:file{[A-Za-z0-9_-]{22}\\.jpg}", async (c) => {
  const id = c.req.param("file").replace(/\.jpg$/, "");
  const clip = await visibleClip(c, id, "t");
  if (!clip || clip.thumb_bytes === 0) return notFound(c);
  const obj = await c.env.BUCKET.get(thumbKey(clip.id));
  if (!obj) return notFound(c);
  const h = new Headers({ "Content-Type": "image/jpeg", ETag: obj.httpEtag });
  cacheHeaders(clip, h);
  return new Response(obj.body, { headers: h });
});
