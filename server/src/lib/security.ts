import type { MiddlewareHandler } from "hono";
import type { AppEnv } from "../env";

/**
 * CSP das páginas: nada de script/estilo inline nem de terceiros. `form-action` libera o
 * loopback porque o login do app termina num redirect para http://127.0.0.1:<porta>.
 */
export const PAGE_CSP = [
  "default-src 'none'",
  "script-src 'self'",
  "style-src 'self'",
  "font-src 'self'",
  "img-src 'self' blob: https://cdn.discordapp.com",
  "media-src 'self' blob:",
  "connect-src 'self'",
  "form-action 'self' http://127.0.0.1:*",
  "base-uri 'none'",
  "frame-ancestors 'none'",
].join("; ");

export const securityHeaders: MiddlewareHandler<AppEnv> = async (c, next) => {
  await next();
  const h = c.res.headers;
  h.set("X-Content-Type-Options", "nosniff");
  h.set("Referrer-Policy", "strict-origin-when-cross-origin");
  h.set("Cross-Origin-Opener-Policy", "same-origin");
  if ((h.get("Content-Type") ?? "").startsWith("text/html")) {
    h.set("Content-Security-Policy", PAGE_CSP);
    h.set("X-Frame-Options", "DENY");
  }
};

/**
 * CSRF: requisição que muda estado e leva COOKIE (sessão, cadastro em andamento) precisa
 * vir desta origem. Sem cookie não há autoridade a roubar (ex.: o app trocando o código
 * pelo token); com Bearer também não (o navegador não manda esse cabeçalho sozinho, e
 * outro site não consegue colocá-lo: não há CORS em lugar nenhum).
 */
export const sameOriginForCookies: MiddlewareHandler<AppEnv> = async (c, next) => {
  const m = c.req.method;
  const unsafe = m !== "GET" && m !== "HEAD" && m !== "OPTIONS";
  if (unsafe && c.req.header("Cookie") && !c.req.header("Authorization")) {
    if (!isSameOrigin(c.req.raw)) {
      return c.text("Origem não permitida", 403);
    }
  }
  await next();
};

export function isSameOrigin(req: Request): boolean {
  const self = new URL(req.url).origin;
  const origin = req.headers.get("Origin");
  if (origin) return origin === self;
  // Sem Origin (raro em navegador moderno num POST): aceita só se o navegador disser same-origin.
  return req.headers.get("Sec-Fetch-Site") === "same-origin";
}

/**
 * Caminho relativo seguro para redirecionar depois do login (evita open redirect).
 * Lista de caracteres permitidos: navegadores ignoram tab/quebra de linha e tratam "\\"
 * como "/", então "/\t/evil" ou "/\\evil" virariam "//evil" (outro site).
 */
export function safeNext(next: string | undefined | null): string {
  if (!next || next.length > 300 || !/^\/[A-Za-z0-9\-._~\/?=&%]*$/.test(next) || next.startsWith("//")) return "/";
  return next;
}
