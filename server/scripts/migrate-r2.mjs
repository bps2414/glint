// Copia os vídeos e miniaturas do R2 da conta antiga para o da conta nova (Fase 5, Parte 8).
// As chaves vêm do banco: clips/<id>/video.mp4 e clips/<id>/thumb.jpg dos clipes prontos.
// Para no primeiro erro; pode rodar de novo (pula o que já foi copiado: .migrate/feitos.txt).
//
//   node scripts/migrate-r2.mjs --from <conta antiga> --to <conta nova>
//        [--from-db clipe] [--from-bucket clipe-clips] [--to-bucket glint-clips]
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { root, step, wrangler } from "./wrangler.mjs";

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  const v = i >= 0 ? process.argv[i + 1] : def;
  if (v === undefined) throw new Error(`falta --${name}`);
  return v;
};
const from = arg("from");
const to = arg("to");
const fromDb = arg("from-db", "clipe");
const fromBucket = arg("from-bucket", "clipe-clips");
const toBucket = arg("to-bucket", "glint-clips");

const dir = join(root, ".migrate");
mkdirSync(dir, { recursive: true });
const doneFile = join(dir, "feitos.txt");
const done = new Set(existsSync(doneFile) ? readFileSync(doneFile, "utf8").split("\n").filter(Boolean) : []);

step(`Clipes no banco "${fromDb}" (conta ${from})`);
const out = wrangler(
  ["d1", "execute", fromDb, "--remote", "--json", "--command",
   "SELECT id, size_bytes, thumb_bytes FROM clips WHERE status = 'ready'"],
  { quiet: true, account: from },
);
const rows = JSON.parse(out)[0].results;
const objects = rows.flatMap((r) => [
  { key: `clips/${r.id}/video.mp4`, size: r.size_bytes },
  ...(r.thumb_bytes > 0 ? [{ key: `clips/${r.id}/thumb.jpg`, size: r.thumb_bytes }] : []),
]);
const total = objects.reduce((s, o) => s + o.size, 0);
console.log(`  ${rows.length} clipes, ${objects.length} objetos, ${(total / 1e9).toFixed(2)} GB; já copiados: ${done.size}`);

let copied = 0;
let bytes = 0;
for (const [i, o] of objects.entries()) {
  if (done.has(o.key)) continue;
  const tmp = join(dir, "objeto.tmp");
  rmSync(tmp, { force: true });
  process.stdout.write(`  [${i + 1}/${objects.length}] ${o.key} … `);
  wrangler(["r2", "object", "get", `${fromBucket}/${o.key}`, "--remote", "--file", tmp], { quiet: true, account: from });
  const got = statSync(tmp).size;
  if (got !== o.size) throw new Error(`${o.key}: baixei ${got} bytes, o banco diz ${o.size}`);
  const type = o.key.endsWith(".mp4") ? "video/mp4" : "image/jpeg";
  wrangler(
    ["r2", "object", "put", `${toBucket}/${o.key}`, "--remote", "--file", tmp, "--content-type", type],
    { quiet: true, account: to },
  );
  rmSync(tmp, { force: true });
  appendFileSync(doneFile, `${o.key}\n`);
  copied++;
  bytes += got;
  console.log("ok");
}
console.log(`\n✔ Copiados agora: ${copied} objetos, ${(bytes / 1e6).toFixed(1)} MB. Total no destino: ${done.size + copied}/${objects.length}.`);
