// Envio de teste pelo navegador, com a mesma API do app (docs/API.md):
// cria o clipe → manda as partes → miniatura → conclui → abre a página do clipe.
const form = document.getElementById("upload");
const statusEl = document.getElementById("status");
const bar = document.getElementById("progress");

const say = (msg) => (statusEl.textContent = msg);

async function api(method, path, body, headers = {}) {
  const res = await fetch(`/api/v1${path}`, { method, body, headers, credentials: "same-origin" });
  if (res.status === 204) return null;
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data?.error?.message || `HTTP ${res.status}`);
  return data;
}

/** Duração/tamanho do vídeo e um quadro em JPEG (640 px) para a miniatura. */
function probe(file) {
  return new Promise((resolve, reject) => {
    const v = document.createElement("video");
    v.muted = true;
    v.preload = "auto";
    v.src = URL.createObjectURL(file);
    v.onerror = () => reject(new Error("O navegador não conseguiu abrir este vídeo (use MP4 H.264)."));
    v.onloadedmetadata = () => {
      v.currentTime = Math.min(1, v.duration / 2);
    };
    v.onseeked = () => {
      const w = 640;
      const h = Math.round((v.videoHeight / v.videoWidth) * w) || 360;
      const cv = document.createElement("canvas");
      cv.width = w;
      cv.height = h;
      cv.getContext("2d").drawImage(v, 0, 0, w, h);
      cv.toBlob(
        (thumb) => {
          URL.revokeObjectURL(v.src);
          resolve({
            duration_ms: Math.max(1, Math.round(v.duration * 1000)),
            width: v.videoWidth,
            height: v.videoHeight,
            thumb,
          });
        },
        "image/jpeg",
        0.85,
      );
    };
  });
}

async function withRetry(fn, tries = 3) {
  for (let i = 1; ; i++) {
    try {
      return await fn();
    } catch (e) {
      if (i >= tries) throw e;
      await new Promise((r) => setTimeout(r, 1000 * i));
    }
  }
}

form.addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const button = form.querySelector("button");
  const file = form.file.files[0];
  if (!file) return;
  button.disabled = true;
  bar.hidden = false;
  bar.value = 0;
  try {
    say("Lendo o vídeo…");
    const info = await probe(file);
    const title = form.title.value.trim() || file.name.replace(/\.mp4$/i, "");
    const { clip, upload } = await api(
      "POST",
      "/clips",
      JSON.stringify({
        title,
        game: form.game.value.trim() || null,
        visibility: form.visibility.value,
        size_bytes: file.size,
        duration_ms: info.duration_ms,
        width: info.width,
        height: info.height,
      }),
      { "Content-Type": "application/json" },
    );

    const done = new Set(upload.parts_done);
    for (let n = 1; n <= upload.part_count; n++) {
      if (done.has(n)) continue;
      const part = file.slice((n - 1) * upload.part_size, n * upload.part_size);
      say(`Enviando parte ${n} de ${upload.part_count}…`);
      await withRetry(() => api("PUT", `/clips/${clip.id}/parts/${n}`, part));
      bar.value = (n / upload.part_count) * 100;
    }
    if (info.thumb) {
      await api("PUT", `/clips/${clip.id}/thumb`, info.thumb, { "Content-Type": "image/jpeg" });
    }
    say("Finalizando…");
    const res = await api("POST", `/clips/${clip.id}/complete`);
    say("Pronto!");
    location.href = res.clip.url;
  } catch (e) {
    say(e.message);
    button.disabled = false;
  }
});
