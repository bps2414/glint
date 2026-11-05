// As versões do app ficam no GitHub Releases. Aqui só sobram rotas de ponte:
// - `/api/v1/app/latest`: apps antigos ainda perguntam ao servidor; mandamos para o `latest.json` do GitHub
//   (mesmo JSON: version, url, file, size, sha256, notes). Não lê o R2.
// - `/baixar`: página de download (link do rodapé do site).
// Rotas públicas: o convite vale para a conta, não para baixar o app.
import { Hono } from "hono";
import type { AppEnv } from "./env";

export const releases = new Hono<AppEnv>();

export const LATEST_JSON_URL = "https://github.com/bps2414/glint/releases/latest/download/latest.json";
export const RELEASES_URL = "https://github.com/bps2414/glint/releases/latest";

releases.get("/api/v1/app/latest", (c) => {
  c.header("Cache-Control", "public, max-age=300");
  return c.redirect(LATEST_JSON_URL, 302);
});

releases.get("/baixar", (c) => c.redirect(RELEASES_URL, 302));
