// Copia as fontes do app (fonte única: crates/app-ui/assets/fonts) para public/static/fonts:
// Inter (texto) e Geist Mono (números), as mesmas da interface.
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "..", "crates", "app-ui", "assets", "fonts");
const dst = join(root, "public", "static", "fonts");
// Fora do repositório completo (ex.: botão "Deploy to Cloudflare", que só clona server/) valem as fontes já versionadas.
if (!existsSync(src)) {
  console.log("fontes do app não encontradas: usando as de public/static/fonts");
  process.exit(0);
}
mkdirSync(dst, { recursive: true });
for (const f of ["Inter-Regular.ttf", "Inter-Medium.ttf", "Inter-SemiBold.ttf", "GeistMono-Medium.ttf", "OFL.txt"]) {
  copyFileSync(join(src, f), join(dst, f));
}
console.log(`fontes copiadas para ${dst}`);
