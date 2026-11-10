// Pacote do servidor para o assistente "Criar meu servidor" do app (server.json, anexado a cada
// release): Worker já empacotado pelo wrangler, migrações, assets (com o hash que a API de assets
// espera) e a configuração do wrangler.jsonc. O app sobe tudo pela API REST da Cloudflare
// (crates/app-cloud/src/cloudflare.rs), sem Node nem wrangler na máquina do usuário.
//
//   node scripts/bundle.mjs <saída.json>
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { hash } from "blake3-wasm";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = process.argv[2];
if (!out) throw new Error("uso: node scripts/bundle.mjs <saída.json>");

// Mesmo leitor de JSONC do deploy.mjs.
function parseJsonc(text) {
  let s = "";
  let inStr = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inStr) {
      s += ch;
      if (ch === "\\") s += text[++i];
      else if (ch === '"') inStr = false;
    } else if (ch === '"') {
      inStr = true;
      s += ch;
    } else if (ch === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      s += "\n";
    } else if (ch === "/" && text[i + 1] === "*") {
      i = text.indexOf("*/", i + 2) + 1;
    } else s += ch;
  }
  return JSON.parse(s.replace(/,(\s*[}\]])/g, "$1"));
}

const config = parseJsonc(readFileSync(join(root, "wrangler.jsonc"), "utf8"));
const version = /SERVER_VERSION = "([^"]+)"/.exec(readFileSync(join(root, "src", "env.ts"), "utf8"))[1];

execFileSync(process.execPath, [join(root, "scripts", "fonts.mjs")], { stdio: "inherit" });

// Worker empacotado (esbuild do wrangler), sem publicar nada.
const tmp = mkdtempSync(join(tmpdir(), "glint-bundle-"));
const files = (dir) =>
  readdirSync(dir, { recursive: true })
    .map((f) => join(dir, f))
    .filter((f) => statSync(f).isFile());
const MODULE_TYPES = {
  ".js": "application/javascript+module",
  ".mjs": "application/javascript+module",
  ".wasm": "application/wasm",
  ".txt": "text/plain",
  ".html": "text/plain",
  ".bin": "application/octet-stream",
};
let modules;
try {
  const bin = join(root, "node_modules", ".bin", process.platform === "win32" ? "wrangler.cmd" : "wrangler");
  execFileSync(bin, ["deploy", "--dry-run", "--outdir", tmp], {
    cwd: root,
    stdio: ["ignore", "inherit", "inherit"],
    shell: process.platform === "win32",
  });
  modules = files(tmp)
    .filter((f) => MODULE_TYPES[extname(f)])
    .map((f) => ({
      name: relative(tmp, f).split(sep).join("/"),
      type: MODULE_TYPES[extname(f)],
      b64: readFileSync(f).toString("base64"),
    }));
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
const main = config.main.replace(/^.*\//, "").replace(/\.tsx?$/, ".js");
if (!modules.some((m) => m.name === main)) throw new Error(`módulo principal ${main} não saiu do wrangler`);

const migrationsDir = join(root, config.d1_databases[0].migrations_dir);
const migrations = readdirSync(migrationsDir)
  .filter((f) => f.endsWith(".sql"))
  .sort()
  .map((name) => ({ name, sql: readFileSync(join(migrationsDir, name), "utf8") }));

// Assets como o wrangler monta (buildAssetManifest/hashFile): `_headers` vai na configuração.
const ASSET_TYPES = {
  ".css": "text/css",
  ".js": "text/javascript",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".webp": "image/webp",
  ".ttf": "font/ttf",
  ".woff2": "font/woff2",
  ".txt": "text/plain",
  ".html": "text/html",
  ".json": "application/json",
};
const publicDir = join(root, config.assets.directory);
const META = new Set(["_headers", "_redirects", ".assetsignore"]);
const assets = files(publicDir)
  .filter((f) => !META.has(relative(publicDir, f)))
  .map((f) => {
    const data = readFileSync(f);
    const b64 = data.toString("base64");
    return {
      path: "/" + relative(publicDir, f).split(sep).join("/"),
      hash: hash(b64 + extname(f).substring(1)).toString("hex").slice(0, 32),
      size: data.length,
      type: ASSET_TYPES[extname(f).toLowerCase()] ?? "application/null",
      b64,
    };
  });
let headers = null;
try {
  headers = readFileSync(join(publicDir, "_headers"), "utf8");
} catch {}

const bundle = {
  version,
  compatibility_date: config.compatibility_date,
  main_module: main,
  modules,
  migrations,
  assets,
  headers,
  crons: config.triggers?.crons ?? [],
  vars: config.vars ?? {},
  names: {
    worker: config.name,
    database: config.d1_databases[0].database_name,
    bucket: config.r2_buckets[0].bucket_name,
  },
};
writeFileSync(out, JSON.stringify(bundle));
console.log(
  `${out}: servidor ${version}, ${modules.length} módulo(s), ${migrations.length} migração(ões), ${assets.length} asset(s), ${(statSync(out).size / 1024).toFixed(0)} KiB`,
);
