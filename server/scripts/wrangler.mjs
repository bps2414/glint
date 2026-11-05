// Utilidades comuns aos scripts que chamam o wrangler (deploy, release, migração).
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const bin = join(root, "node_modules", ".bin", process.platform === "win32" ? "wrangler.cmd" : "wrangler");

/** Roda o wrangler. `account` troca de conta (CLOUDFLARE_ACCOUNT_ID) só nesta chamada. */
export function wrangler(args, { quiet = false, allowFail = false, account } = {}) {
  const env = { ...process.env };
  if (account) env.CLOUDFLARE_ACCOUNT_ID = account;
  try {
    return execFileSync(bin, args, {
      cwd: root,
      env,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: quiet ? ["ignore", "pipe", "pipe"] : ["ignore", "pipe", "inherit"],
      shell: process.platform === "win32",
    });
  } catch (e) {
    if (allowFail) return null;
    throw e;
  }
}

export const step = (s) => console.log(`\n▸ ${s}`);
