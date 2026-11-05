// Deploy completo e repetível: cria D1/R2 se faltarem, aplica migrações, publica com segredos.
//
//   Local:   npx wrangler login  &&  DISCORD_CLIENT_ID=... DISCORD_CLIENT_SECRET=... ADMIN_DISCORD_IDS=... npm run deploy
//   CI:      job "Deploy na Cloudflare" do workflow "Servidor" (.github/workflows/server.yml).
//   Botão "Deploy to Cloudflare" (Workers Builds, WORKERS_CI=1): D1/R2 já foram criados pelo botão a partir do
//            wrangler.jsonc; aqui só migra e publica.
//
// Variáveis lidas do ambiente (todas opcionais depois do primeiro deploy, exceto quando mudam):
//   DISCORD_CLIENT_ID, ADMIN_DISCORD_IDS, INVITE_REQUIRED,
//   USER_QUOTA_GB, STORAGE_LIMIT_GB, SERVER_LANG           → vars do Worker
//   DISCORD_CLIENT_SECRET, SIGNING_KEY                     → segredos (SIGNING_KEY é gerado se faltar)
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const bin = join(root, "node_modules", ".bin", process.platform === "win32" ? "wrangler.cmd" : "wrangler");
const deployConfig = join(root, "wrangler.deploy.jsonc");
const secretsFile = join(root, ".deploy-secrets.json");

function wrangler(args, { quiet = false, allowFail = false } = {}) {
  try {
    return execFileSync(bin, args, {
      cwd: root,
      encoding: "utf8",
      stdio: quiet ? ["ignore", "pipe", "pipe"] : ["ignore", "pipe", "inherit"],
      shell: process.platform === "win32",
    });
  } catch (e) {
    if (allowFail) return null;
    throw e;
  }
}

/** JSONC → objeto (remove comentários fora de strings e vírgulas finais). */
function parseJsonc(text) {
  let out = "";
  let inStr = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inStr) {
      out += ch;
      if (ch === "\\") out += text[++i];
      else if (ch === '"') inStr = false;
    } else if (ch === '"') {
      inStr = true;
      out += ch;
    } else if (ch === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      out += "\n";
    } else if (ch === "/" && text[i + 1] === "*") {
      i = text.indexOf("*/", i + 2) + 1;
    } else out += ch;
  }
  return JSON.parse(out.replace(/,(\s*[}\]])/g, "$1"));
}

const step = (s) => console.log(`\n▸ ${s}`);

if (process.env.WORKERS_CI) {
  step("Migrações");
  wrangler(["d1", "migrations", "apply", "DB", "--remote"]);
  step("Deploy");
  process.stdout.write(wrangler(["deploy"]));
  process.exit(0);
}

const config = parseJsonc(readFileSync(join(root, "wrangler.jsonc"), "utf8"));
const db = config.d1_databases[0];
const bucket = config.r2_buckets[0].bucket_name;

step("Fontes");
execFileSync(process.execPath, [join(root, "scripts", "fonts.mjs")], { stdio: "inherit" });

step(`Banco D1 "${db.database_name}"`);
const findDb = () =>
  JSON.parse(wrangler(["d1", "list", "--json"], { quiet: true })).find((d) => d.name === db.database_name);
let found = findDb();
if (!found) {
  console.log("  não existe: criando");
  wrangler(["d1", "create", db.database_name]);
  found = findDb();
}
if (!found?.uuid) throw new Error("não consegui achar o id do banco D1");
console.log(`  id ${found.uuid}`);

step(`Bucket R2 "${bucket}"`);
if (wrangler(["r2", "bucket", "info", bucket], { quiet: true, allowFail: true }) === null) {
  console.log("  não existe: criando");
  wrangler(["r2", "bucket", "create", bucket]);
} else console.log("  ok");

// Config de deploy = wrangler.jsonc + id do banco + vars vindas do ambiente.
const vars = { ...config.vars };
for (const k of ["DISCORD_CLIENT_ID", "ADMIN_DISCORD_IDS", "INVITE_REQUIRED", "USER_QUOTA_GB", "STORAGE_LIMIT_GB", "SERVER_LANG"]) {
  if (process.env[k] !== undefined && process.env[k] !== "") vars[k] = process.env[k].trim();
}
if (!vars.DISCORD_CLIENT_ID) throw new Error("defina DISCORD_CLIENT_ID (Discord Developer Portal → OAuth2)");
if (!vars.ADMIN_DISCORD_IDS) console.warn("  aviso: ADMIN_DISCORD_IDS vazio — em servidor sem usuários, o primeiro login vira admin");
writeFileSync(
  deployConfig,
  JSON.stringify({ ...config, vars, d1_databases: [{ ...db, database_id: found.uuid }] }, null, 2),
);

step("Migrações");
wrangler(["d1", "migrations", "apply", "DB", "--remote", "--config", deployConfig]);

step("Segredos");
const existing = wrangler(["secret", "list", "--config", deployConfig], { quiet: true, allowFail: true });
const have = new Set(existing ? JSON.parse(existing).map((s) => s.name) : []);
const secrets = {};
if (process.env.DISCORD_CLIENT_SECRET) secrets.DISCORD_CLIENT_SECRET = process.env.DISCORD_CLIENT_SECRET.trim();
else if (!have.has("DISCORD_CLIENT_SECRET")) throw new Error("defina DISCORD_CLIENT_SECRET no primeiro deploy");
if (process.env.SIGNING_KEY) secrets.SIGNING_KEY = process.env.SIGNING_KEY.trim();
else if (!have.has("SIGNING_KEY")) {
  console.log("  SIGNING_KEY não existe: gerando uma aleatória");
  secrets.SIGNING_KEY = randomBytes(32).toString("base64url");
}
console.log(`  enviando: ${Object.keys(secrets).join(", ") || "(nenhum novo)"}`);

step("Deploy");
const args = ["deploy", "--config", deployConfig];
try {
  if (Object.keys(secrets).length) {
    writeFileSync(secretsFile, JSON.stringify(secrets), { mode: 0o600 });
    args.push("--secrets-file", secretsFile);
  }
  const out = wrangler(args);
  process.stdout.write(out);
  const url = /https:\/\/[^\s]+\.workers\.dev/.exec(out)?.[0];
  if (url) {
    console.log(`\n✔ No ar: ${url}`);
    console.log(`  No Discord Developer Portal → OAuth2 → Redirects, cadastre:\n  ${url}/auth/discord/callback`);
  }
} finally {
  rmSync(secretsFile, { force: true });
}
