import type { DiscordApi } from "./auth/discord";
import { APP_NAME } from "./brand";

/** Bindings e variáveis do Worker (wrangler.jsonc + segredos). */
export interface Bindings {
  DB: D1Database;
  BUCKET: R2Bucket;
  INVITE_REQUIRED: string;
  ADMIN_DISCORD_IDS: string;
  DISCORD_CLIENT_ID: string;
  /** Cota por usuário em GB. Ausente/0 = sem cota por usuário (padrão). */
  USER_QUOTA_GB: string;
  /** Trava global: a soma de todos os clipes não passa disso (GB, padrão 9,5). */
  STORAGE_LIMIT_GB: string;
  /** Língua das páginas e mensagens: "en" (padrão) ou "pt-BR". */
  SERVER_LANG?: string;
  MAX_CLIP_MB: string;
  /** Nome do servidor mostrado no convite e no app (padrão: o nome do app). */
  SERVER_NAME?: string;
  /** Segredo (wrangler secret put). Só com login Discord. */
  DISCORD_CLIENT_SECRET?: string;
  /** Segredo: chave HMAC das URLs temporárias de clipes privados. */
  SIGNING_KEY: string;
  /**
   * Segredo opcional: quem souber cria o admin pelo POST /api/v1/setup (servidor novo, sem usuários).
   * Definido, o primeiro login Discord deixa de virar admin.
   */
  SETUP_TOKEN?: string;
}

/** Versão do servidor (acompanha o app, Cargo.toml) e da API que ele fala. */
export const SERVER_VERSION = "1.0.0-rc.3";
export const API_VERSION = 1;

export interface User {
  id: string;
  /** `null` = conta criada por convite, sem Discord. */
  discord_id: string | null;
  /** Nome mostrado no servidor (escolhido no convite ou vindo do Discord). */
  name: string;
  username: string;
  display_name: string | null;
  avatar: string | null;
  role: "user" | "admin";
  quota_bytes: number;
  disabled: number;
  created_at: number;
}

export interface Session {
  token_hash: string;
  kind: "web" | "app";
  expires_at: number;
  last_used_at: number;
}

/** Dependências externas (trocadas nos testes). */
export interface Deps {
  discord: DiscordApi;
  now: () => number;
}

export type AppEnv = {
  Bindings: Bindings;
  Variables: {
    deps: Deps;
    config: Config;
    user: User | null;
    session: Session | null;
    /** Como o usuário se autenticou nesta requisição. */
    authVia: "cookie" | "bearer" | null;
  };
};

export interface Config {
  /** Login Discord ligado (DISCORD_CLIENT_ID configurado). */
  discordLogin: boolean;
  /** Nome do servidor (SERVER_NAME ou o nome do app). */
  serverName: string;
  inviteRequired: boolean;
  adminDiscordIds: Set<string>;
  /** Cota padrão por usuário; 0 = sem cota por usuário. */
  userQuotaBytes: number;
  /** Limite da soma de todos os clipes (uploads pendentes contam). */
  storageLimitBytes: number;
  maxClipBytes: number;
}

const GB = 1024 ** 3;
const MB = 1024 ** 2;

export function readConfig(env: Bindings): Config {
  const num = (v: string | undefined, fallback: number) => {
    const n = Number(v);
    return Number.isFinite(n) && n > 0 ? n : fallback;
  };
  return {
    discordLogin: !!(env.DISCORD_CLIENT_ID ?? "").trim(),
    serverName: (env.SERVER_NAME ?? "").trim().slice(0, 60) || APP_NAME,
    // Qualquer coisa diferente de "false" mantém o portão (falhar fechado).
    inviteRequired: (env.INVITE_REQUIRED ?? "").trim().toLowerCase() !== "false",
    adminDiscordIds: new Set(
      (env.ADMIN_DISCORD_IDS ?? "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => /^\d{5,25}$/.test(s)),
    ),
    userQuotaBytes: Math.round(num(env.USER_QUOTA_GB, 0) * GB),
    storageLimitBytes: Math.round(num(env.STORAGE_LIMIT_GB, 9.5) * GB),
    maxClipBytes: Math.round(num(env.MAX_CLIP_MB, 500) * MB),
  };
}

/** Tamanho das partes do upload (todas iguais, menos a última; R2 exige >= 5 MiB). */
export const PART_SIZE = 8 * MB;
/** Miniatura JPEG enviada pelo app. */
export const MAX_THUMB_BYTES = 1 * MB;
/** Upload não concluído some depois disso (e libera a cota). */
export const UPLOAD_TTL_MS = 24 * 3600_000;
export const WEB_SESSION_MS = 30 * 24 * 3600_000;
export const APP_SESSION_MS = 180 * 24 * 3600_000;
/** Código curto para entrar com o mesmo usuário em outro PC. */
export const LINK_CODE_MS = 10 * 60_000;
/** Código de uso único com que o app abre o site já logado. */
export const WEB_CODE_MS = 60_000;
/** URL temporária de vídeo/miniatura privados. */
export const SIGNED_URL_MS = 3600_000;
