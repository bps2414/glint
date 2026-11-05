import { createApp } from "./app";
import { realDiscord } from "./auth/discord";
import { cleanup } from "./cron";
import type { Bindings } from "./env";

const app = createApp({ discord: realDiscord, now: () => Date.now() });

export default {
  fetch: app.fetch,
  async scheduled(_controller, env, ctx) {
    ctx.waitUntil(cleanup(env, Date.now()).then((r) => console.log("faxina", r)));
  },
} satisfies ExportedHandler<Bindings>;
