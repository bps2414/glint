// API v1 dos clipes. Upload em partes pelo Worker para o multipart do R2 (retomável).
// Contrato: docs/API.md.
import { Hono, type Context } from "hono";
import { z } from "zod";
import type { AppEnv } from "../env";
import { MAX_THUMB_BYTES, PART_SIZE } from "../env";
import { avatarUrl } from "../auth/discord";
import { t } from "../i18n";
import { randomId } from "../lib/crypto";
import {
  clipDto,
  isClipId,
  partCount,
  partLength,
  thumbKey,
  totalBytes,
  usedBytes,
  userQuota,
  videoKey,
  type ClipRow,
} from "../lib/clips";
import { allow } from "../lib/ratelimit";

type Ctx = Context<AppEnv>;
type ErrStatus = 400 | 401 | 403 | 404 | 409 | 411 | 413 | 422 | 429 | 500 | 507;

export function apiError(c: Ctx, status: ErrStatus, code: string, message: string, extra?: object) {
  return c.json({ error: { code, message, ...extra } }, status);
}

export const api = new Hono<AppEnv>();

api.use("*", async (c, next) => {
  if (!c.var.user) return apiError(c, 401, "unauthorized", t.api.unauthorized);
  await next();
});

const text = (max: number) =>
  z
    .string()
    .trim()
    .max(max)
    // Sem caracteres de controle (quebra de linha etc.) em títulos.
    .refine((s) => !/[\u0000-\u001f\u007f]/.test(s));
const visibility = z.enum(["private", "unlisted", "public"]);

const createSchema = z.object({
  title: text(100),
  game: text(60).nullish(),
  visibility: visibility.default("unlisted"),
  size_bytes: z.int().min(1),
  duration_ms: z.int().min(1).max(30 * 60_000),
  width: z.int().min(16).max(7680),
  height: z.int().min(16).max(4320),
});

const patchSchema = z
  .object({ title: text(100).optional(), visibility: visibility.optional() })
  .refine((o) => o.title !== undefined || o.visibility !== undefined);

async function jsonBody(c: Ctx): Promise<unknown> {
  return c.req.json().catch(() => undefined);
}

async function ownClip(c: Ctx): Promise<ClipRow | null> {
  const id = c.req.param("id");
  if (!isClipId(id)) return null;
  return c.env.DB.prepare("SELECT * FROM clips WHERE id = ? AND user_id = ?")
    .bind(id, c.var.user!.id)
    .first<ClipRow>();
}

async function uploadState(c: Ctx, clip: ClipRow) {
  const { results } = await c.env.DB.prepare(
    "SELECT part_number FROM clip_parts WHERE clip_id = ? ORDER BY part_number",
  )
    .bind(clip.id)
    .all<{ part_number: number }>();
  return {
    part_size: PART_SIZE,
    part_count: partCount(clip.size_bytes),
    parts_done: results.map((r) => r.part_number),
  };
}

api.get("/me", async (c) => {
  const u = c.var.user!;
  return c.json({
    user: {
      id: u.id,
      username: u.username,
      display_name: u.display_name,
      avatar_url: avatarUrl(u.discord_id, u.avatar),
      role: u.role,
    },
    // quota_bytes = 0: servidor sem cota por usuário.
    quota: { used_bytes: await usedBytes(c.env.DB, u.id), quota_bytes: userQuota(c.var.config, u) },
    limits: { max_clip_bytes: c.var.config.maxClipBytes, part_size: PART_SIZE, max_thumb_bytes: MAX_THUMB_BYTES },
  });
});

api.post("/clips", async (c) => {
  const user = c.var.user!;
  const parsed = createSchema.safeParse(await jsonBody(c));
  if (!parsed.success) return apiError(c, 400, "invalid", t.api.invalid);
  const b = parsed.data;
  if (b.size_bytes > c.var.config.maxClipBytes) {
    return apiError(c, 413, "too_large", t.api.tooLarge, { max_clip_bytes: c.var.config.maxClipBytes });
  }
  if (!(await allow(c, `upload:${user.id}`, 60, 3600))) return apiError(c, 429, "rate_limited", t.api.tooMany);

  const id = randomId();
  const now = c.var.deps.now();
  const mp = await c.env.BUCKET.createMultipartUpload(videoKey(id), {
    httpMetadata: { contentType: "video/mp4" },
  });
  // Reserva o espaço na mesma instrução que cria o clipe (duas criações ao mesmo tempo não
  // passam juntas do limite). Uploads pendentes contam até concluir ou vencer (a faxina
  // apaga os abandonados e libera o espaço). Vale a trava global do servidor e, se ligada,
  // a cota do usuário (`quota = 0`: sem cota por usuário).
  const quota = userQuota(c.var.config, user);
  const limit = c.var.config.storageLimitBytes;
  const res = await c.env.DB.prepare(
    `INSERT INTO clips (id, user_id, title, game, visibility, status, size_bytes, duration_ms, width, height, upload_id, created_at)
     SELECT ?, ?, ?, ?, ?, 'uploading', ?, ?, ?, ?, ?, ?
      WHERE (SELECT COALESCE(SUM(size_bytes + thumb_bytes), 0) FROM clips) + ? <= ?
        AND (? = 0 OR (SELECT COALESCE(SUM(size_bytes + thumb_bytes), 0) FROM clips WHERE user_id = ?) + ? <= ?)`,
  )
    .bind(
      id,
      user.id,
      b.title || t.untitled,
      b.game || null,
      b.visibility,
      b.size_bytes,
      b.duration_ms,
      b.width,
      b.height,
      mp.uploadId,
      now,
      b.size_bytes,
      limit,
      quota,
      user.id,
      b.size_bytes,
      quota,
    )
    .run();
  if (!res.meta.changes) {
    await mp.abort();
    const used = await usedBytes(c.env.DB, user.id);
    if (quota > 0 && used + b.size_bytes > quota) {
      return apiError(c, 409, "quota_exceeded", t.api.quota, { used_bytes: used, quota_bytes: quota });
    }
    // Servidor cheio (trava global): 507, o app mostra o motivo e não tenta de novo.
    return apiError(c, 507, "storage_full", t.api.storageFull, {
      used_bytes: await totalBytes(c.env.DB),
      limit_bytes: limit,
    });
  }
  const clip = (await c.env.DB.prepare("SELECT * FROM clips WHERE id = ?").bind(id).first<ClipRow>())!;
  return c.json({ clip: await clipDto(c, clip), upload: await uploadState(c, clip) }, 201);
});

api.get("/clips", async (c) => {
  const limit = 50;
  const before = Number(c.req.query("before") ?? Number.MAX_SAFE_INTEGER);
  const { results } = await c.env.DB.prepare(
    "SELECT * FROM clips WHERE user_id = ? AND created_at < ? ORDER BY created_at DESC LIMIT ?",
  )
    .bind(c.var.user!.id, Number.isFinite(before) ? before : Number.MAX_SAFE_INTEGER, limit)
    .all<ClipRow>();
  const clips = await Promise.all(results.map((r) => clipDto(c, r)));
  const last = results.at(-1);
  return c.json({ clips, next_before: results.length === limit && last ? last.created_at : null });
});

api.get("/clips/:id", async (c) => {
  const clip = await ownClip(c);
  if (!clip) return apiError(c, 404, "not_found", t.api.notFound);
  return c.json({
    clip: await clipDto(c, clip),
    upload: clip.status === "uploading" ? await uploadState(c, clip) : null,
  });
});

api.put("/clips/:id/parts/:n", async (c) => {
  const clip = await ownClip(c);
  if (!clip) return apiError(c, 404, "not_found", t.api.notFound);
  if (clip.status !== "uploading" || !clip.upload_id) return apiError(c, 409, "not_uploading", t.api.notUploading);
  const n = Number(c.req.param("n"));
  const count = partCount(clip.size_bytes);
  if (!Number.isInteger(n) || n < 1 || n > count) return apiError(c, 400, "invalid", t.api.invalid);

  const len = c.req.header("Content-Length");
  if (len === undefined || !/^\d+$/.test(len)) return apiError(c, 411, "length_required", t.api.lengthRequired);
  const expected = partLength(clip.size_bytes, n);
  if (Number(len) !== expected || !c.req.raw.body) {
    return apiError(c, 400, "part_size", t.api.partSize, { expected_bytes: expected });
  }

  const mp = c.env.BUCKET.resumeMultipartUpload(videoKey(clip.id), clip.upload_id);
  const part = await mp.uploadPart(n, c.req.raw.body);
  await c.env.DB.prepare(
    `INSERT INTO clip_parts (clip_id, part_number, etag) VALUES (?, ?, ?)
     ON CONFLICT (clip_id, part_number) DO UPDATE SET etag = excluded.etag`,
  )
    .bind(clip.id, n, part.etag)
    .run();
  return c.json({ part_number: n });
});

api.put("/clips/:id/thumb", async (c) => {
  const clip = await ownClip(c);
  if (!clip) return apiError(c, 404, "not_found", t.api.notFound);
  const len = Number(c.req.header("Content-Length"));
  if (!Number.isInteger(len) || len < 4 || len > MAX_THUMB_BYTES) return apiError(c, 400, "bad_thumb", t.api.badThumb);
  const buf = new Uint8Array(await c.req.arrayBuffer());
  if (buf.byteLength !== len || buf[0] !== 0xff || buf[1] !== 0xd8 || buf[2] !== 0xff) {
    return apiError(c, 400, "bad_thumb", t.api.badThumb);
  }
  await c.env.BUCKET.put(thumbKey(clip.id), buf, { httpMetadata: { contentType: "image/jpeg" } });
  await c.env.DB.prepare("UPDATE clips SET thumb_bytes = ? WHERE id = ?").bind(buf.byteLength, clip.id).run();
  return c.body(null, 204);
});

api.post("/clips/:id/complete", async (c) => {
  const clip = await ownClip(c);
  if (!clip) return apiError(c, 404, "not_found", t.api.notFound);
  if (clip.status === "ready") return c.json({ clip: await clipDto(c, clip) });
  if (!clip.upload_id) return apiError(c, 409, "not_uploading", t.api.notUploading);

  const { results } = await c.env.DB.prepare(
    "SELECT part_number, etag FROM clip_parts WHERE clip_id = ? ORDER BY part_number",
  )
    .bind(clip.id)
    .all<{ part_number: number; etag: string }>();
  const count = partCount(clip.size_bytes);
  if (results.length !== count) {
    return apiError(c, 409, "missing_parts", t.api.missingParts, {
      parts_done: results.map((r) => r.part_number),
      part_count: count,
    });
  }

  const key = videoKey(clip.id);
  const mp = c.env.BUCKET.resumeMultipartUpload(key, clip.upload_id);
  await mp.complete(results.map((r) => ({ partNumber: r.part_number, etag: r.etag })));

  // Confere o que chegou: tamanho exato e assinatura de MP4 ("ftyp" nos bytes 4..8).
  const head = await c.env.BUCKET.get(key, { range: { offset: 0, length: 12 } });
  const bytes = head ? new Uint8Array(await head.arrayBuffer()) : new Uint8Array();
  const isMp4 = new TextDecoder().decode(bytes.subarray(4, 8)) === "ftyp";
  if (!head || head.size !== clip.size_bytes || !isMp4) {
    await c.env.BUCKET.delete([key, thumbKey(clip.id)]);
    await c.env.DB.prepare("DELETE FROM clips WHERE id = ?").bind(clip.id).run();
    return apiError(c, 422, "bad_video", t.api.badVideo);
  }

  const now = c.var.deps.now();
  await c.env.DB.batch([
    c.env.DB.prepare("UPDATE clips SET status = 'ready', upload_id = NULL, ready_at = ? WHERE id = ?").bind(now, clip.id),
    c.env.DB.prepare("DELETE FROM clip_parts WHERE clip_id = ?").bind(clip.id),
  ]);
  const ready = { ...clip, status: "ready" as const, upload_id: null, ready_at: now };
  return c.json({ clip: await clipDto(c, ready) });
});

api.patch("/clips/:id", async (c) => {
  const clip = await ownClip(c);
  if (!clip) return apiError(c, 404, "not_found", t.api.notFound);
  const parsed = patchSchema.safeParse(await jsonBody(c));
  if (!parsed.success) return apiError(c, 400, "invalid", t.api.invalid);
  const title = parsed.data.title === undefined ? clip.title : parsed.data.title || t.untitled;
  const vis = parsed.data.visibility ?? clip.visibility;
  await c.env.DB.prepare("UPDATE clips SET title = ?, visibility = ? WHERE id = ?").bind(title, vis, clip.id).run();
  return c.json({ clip: await clipDto(c, { ...clip, title, visibility: vis }) });
});

api.delete("/clips/:id", async (c) => {
  const clip = await ownClip(c);
  if (!clip) return apiError(c, 404, "not_found", t.api.notFound);
  await deleteClip(c.env, clip);
  return c.body(null, 204);
});

/** Apaga objetos do R2 e a linha (usado também pela faxina). */
export async function deleteClip(env: { DB: D1Database; BUCKET: R2Bucket }, clip: ClipRow) {
  if (clip.status === "uploading" && clip.upload_id) {
    await env.BUCKET.resumeMultipartUpload(videoKey(clip.id), clip.upload_id)
      .abort()
      .catch(() => {});
  }
  await env.BUCKET.delete([videoKey(clip.id), thumbKey(clip.id)]);
  await env.DB.prepare("DELETE FROM clips WHERE id = ?").bind(clip.id).run();
}
