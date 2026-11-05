// Aleatoriedade, hashes e HMAC (Web Crypto do Workers).

const enc = new TextEncoder();

export function b64url(bytes: ArrayBuffer | Uint8Array): string {
  const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let s = "";
  for (const b of u8) s += String.fromCharCode(b);
  return btoa(s).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

/** Token aleatório em base64url (`bytes` de entropia). */
export function randomToken(bytes = 32): string {
  return b64url(crypto.getRandomValues(new Uint8Array(bytes)));
}

/** ID público de 128 bits (22 caracteres base64url). */
export function randomId(): string {
  return randomToken(16);
}

export async function sha256(text: string): Promise<string> {
  return b64url(await crypto.subtle.digest("SHA-256", enc.encode(text)));
}

const keyCache = new Map<string, Promise<CryptoKey>>();
function hmacKey(secret: string): Promise<CryptoKey> {
  let k = keyCache.get(secret);
  if (!k) {
    k = crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
      "sign",
    ]);
    keyCache.set(secret, k);
  }
  return k;
}

export async function hmac(secret: string, message: string): Promise<string> {
  return b64url(await crypto.subtle.sign("HMAC", await hmacKey(secret), enc.encode(message)));
}

/** Comparação em tempo constante (para tokens/assinaturas do mesmo formato). */
export function safeEqual(a: string, b: string): boolean {
  const x = enc.encode(a);
  const y = enc.encode(b);
  if (x.byteLength !== y.byteLength) return false;
  return crypto.subtle.timingSafeEqual(x, y);
}

/** Formato dos tokens/IDs que aceitamos de fora (evita lixo nas consultas). */
export function isToken(s: unknown, min = 16, max = 128): s is string {
  return typeof s === "string" && s.length >= min && s.length <= max && /^[A-Za-z0-9_-]+$/.test(s);
}
