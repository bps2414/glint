// "Copiar link": qualquer botão com data-copy (o texto fica em [data-label], se houver).
for (const b of document.querySelectorAll("[data-copy]")) {
  const target = b.querySelector("[data-label]") || b;
  b.addEventListener("click", async () => {
    const label = target.textContent;
    try {
      await navigator.clipboard.writeText(b.dataset.copy);
      target.textContent = b.dataset.copied || "OK";
      setTimeout(() => (target.textContent = label), 1500);
    } catch {
      prompt("Copie o link:", b.dataset.copy);
    }
  });
}

// Player da página do clipe: botão de vidro no meio; tocando, ele e a duração somem.
for (const p of document.querySelectorAll("[data-player]")) {
  const video = p.querySelector("video");
  const play = p.querySelector("[data-play]");
  if (!video || !play) continue;
  // Parado mostra só o vidro; os controles do navegador aparecem depois do primeiro play.
  video.controls = false;
  const sync = () => p.classList.toggle("playing", !video.paused && !video.ended);
  play.addEventListener("click", () => {
    video.controls = true;
    video.play();
  });
  video.addEventListener("play", sync);
  video.addEventListener("pause", sync);
  video.addEventListener("ended", sync);
}
