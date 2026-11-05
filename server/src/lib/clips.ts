import type { Context } from "hono";
import type { AppEnv, Bindings, Config, User } from "../env";
import { PART_SIZE, SIGNED_URL_MS } from "../env";
import { hmac, isToken, safeEqual } from "./crypto";

export type Visibility = "private" | "unlisted" | "public";

export interface ClipRow {
  id: string;
  user_id: string;
  title: string;
  game: string | null;
  visibility: Visibility;
  status: "uploading" | "ready";
  size_bytes: number;
  thumb_bytes: number;
  duration_ms: number;
  width: number;
  height: number;
  upload_id: string | null;
  created_at: number;
  ready_at: number | null;
}

export const videoKey = (id: string) => `clips/${id}/video.mp4`;
export const thumbKey = (id: string) => `clips/${id}/thumb.jpg`;

export const isClipId = (s: unknown): s is string => isToken(s, 22, 22);

export function partCount(size: number): number {
  return Math.max(1, Math.ceil(size / PART_SIZE));
}

/** Tamanho esperado da parte `n` (1..count). */
export function partLength(size: number, n: number): number {
  const count = partCount(size);
  return n < count ? PART_SIZE : size - (count - 1) * PART_SIZE;
}

function signingKey(env: Bindings): string {
  // Sem chave forte não há como proteger os privados: falha em vez de assinar com lixo.
  if (!env.SIGNING_KEY || env.SIGNING_KEY.length < 32) throw new Error("SIGNING_KEY ausente ou curta");
  return env.SIGNING_KEY;
}

type MediaKind = "v" | "t";

/** Query string temporária (`exp`, `sig`) para mídia de clipe privado. */
export async function signMedia(env: Bindings, kind: MediaKind, id: string, now: number): Promise<string> {
  const exp = now + SIGNED_URL_MS;
  const sig = await hmac(signingKey(env), `${kind}:${id}:${exp}`);
  return `?exp=${exp}&sig=${sig}`;
}

export async function verifyMedia(
  env: Bindings,
  kind: MediaKind,
  id: string,
  exp: string | undefined,
  sig: string | undefined,
  now: number,
): Promise<boolean> {
  if (!exp || !/^\d{1,16}$/.test(exp) || !isToken(sig, 43, 43)) return false;
  if (Number(exp) <= now) return false;
  return safeEqual(await hmac(signingKey(env), `${kind}:${id}:${exp}`), sig);
}

/** URLs públicas (ou temporárias, se privado) do clipe. */
export async function clipUrls(c: Context<AppEnv>, clip: ClipRow) {
  const origin = new URL(c.req.url).origin;
  const now = c.var.deps.now();
  const q = async (k: MediaKind) => (clip.visibility === "private" ? await signMedia(c.env, k, clip.id, now) : "");
  return {
    page: `${origin}/c/${clip.id}`,
    video: `${origin}/v/${clip.id}.mp4${await q("v")}`,
    thumb: clip.thumb_bytes > 0 ? `${origin}/t/${clip.id}.jpg${await q("t")}` : null,
  };
}

/** Clipe como a API devolve (contrato em docs/API.md). */
export async function clipDto(c: Context<AppEnv>, clip: ClipRow) {
  const urls = clip.status === "ready" ? await clipUrls(c, clip) : null;
  return {
    id: clip.id,
    title: clip.title,
    game: clip.game,
    visibility: clip.visibility,
    status: clip.status,
    size_bytes: clip.size_bytes,
    duration_ms: clip.duration_ms,
    width: clip.width,
    height: clip.height,
    created_at: clip.created_at,
    url: urls?.page ?? null,
    video_url: urls?.video ?? null,
    thumb_url: urls?.thumb ?? null,
  };
}

/** Cota efetiva do usuário em bytes; 0 = sem cota por usuário (desligada no servidor). */
export function userQuota(config: Config, user: Pick<User, "quota_bytes">): number {
  if (config.userQuotaBytes === 0) return 0;
  return user.quota_bytes > 0 ? user.quota_bytes : config.userQuotaBytes;
}

/** Soma de todos os clipes (vídeo + miniatura), prontos ou em upload. */
export async function totalBytes(db: D1Database): Promise<number> {
  const r = await db
    .prepare("SELECT COALESCE(SUM(size_bytes + thumb_bytes), 0) AS used FROM clips")
    .first<{ used: number }>();
  return r?.used ?? 0;
}

export async function usedBytes(db: D1Database, userId: string): Promise<number> {
  const r = await db
    .prepare("SELECT COALESCE(SUM(size_bytes + thumb_bytes), 0) AS used FROM clips WHERE user_id = ?")
    .bind(userId)
    .first<{ used: number }>();
  return r?.used ?? 0;
}
