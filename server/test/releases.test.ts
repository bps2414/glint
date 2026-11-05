import { describe, expect, it } from "vitest";
import { call } from "./helpers";

describe("versões do app (ponte para o GitHub Releases)", () => {
  it("latest redireciona para o latest.json do GitHub", async () => {
    const r = await call("/api/v1/app/latest", { redirect: "manual" });
    expect(r.status).toBe(302);
    expect(r.headers.get("Location")).toBe("https://github.com/bps2414/glint/releases/latest/download/latest.json");
  });

  it("/baixar aponta para a página de releases", async () => {
    const r = await call("/baixar", { redirect: "manual" });
    expect(r.status).toBe(302);
    expect(r.headers.get("Location")).toBe("https://github.com/bps2414/glint/releases/latest");
  });

  it("o instalador não é mais servido pelo Worker", async () => {
    expect((await call("/baixar/Glint-Setup-1.0.0.exe")).status).toBe(404);
  });
});
