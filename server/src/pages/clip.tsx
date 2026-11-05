/** @jsxImportSource hono/jsx */
import { APP_NAME, THEME_COLOR } from "../brand";
import type { User } from "../env";
import { t } from "../i18n";
import type { ClipRow } from "../lib/clips";
import { formatDate, formatDuration } from "../lib/format";
import { Layout } from "./layout";

/**
 * Página do clipe. Para o embed do Discord: og:video aponta direto para o MP4 (H.264),
 * com tipo e tamanho; og:image é a miniatura. Privado não tem meta tags (nem embed).
 */
export function ClipPage(props: {
  clip: ClipRow;
  urls: { page: string; video: string; thumb: string | null };
  owner: string;
  viewer: User | null;
}) {
  const { clip, urls } = props;
  const isPrivate = clip.visibility === "private";
  const desc = [clip.game, formatDuration(clip.duration_ms), props.owner].filter(Boolean).join(" · ");
  const head = isPrivate ? (
    <meta name="robots" content="noindex" />
  ) : (
    <>
      {clip.visibility !== "public" && <meta name="robots" content="noindex" />}
      <meta property="og:site_name" content={APP_NAME} />
      <meta property="og:type" content="video.other" />
      <meta property="og:title" content={clip.title} />
      <meta property="og:description" content={desc} />
      <meta property="og:url" content={urls.page} />
      <meta property="og:video" content={urls.video} />
      <meta property="og:video:url" content={urls.video} />
      <meta property="og:video:secure_url" content={urls.video} />
      <meta property="og:video:type" content="video/mp4" />
      <meta property="og:video:width" content={String(clip.width)} />
      <meta property="og:video:height" content={String(clip.height)} />
      {urls.thumb && <meta property="og:image" content={urls.thumb} />}
      {/* Sem og:image:width/height: os clipes novos mandam 1200×630 (composta no app) e os
          antigos, a miniatura no tamanho do vídeo; o Discord mede a imagem sozinho. */}
      <meta name="twitter:card" content="player" />
      <meta name="twitter:title" content={clip.title} />
      <meta name="twitter:player" content={urls.video} />
      <meta name="twitter:player:stream" content={urls.video} />
      <meta name="twitter:player:stream:content_type" content="video/mp4" />
      <meta name="twitter:player:width" content={String(clip.width)} />
      <meta name="twitter:player:height" content={String(clip.height)} />
      {urls.thumb && <meta name="twitter:image" content={urls.thumb} />}
      <meta name="theme-color" content={THEME_COLOR} />
    </>
  );

  const initial = (props.owner.match(/[\p{L}\p{N}]/u)?.[0] ?? "?").toUpperCase();
  return (
    <Layout title={clip.title} head={head} user={props.viewer} scripts={["clip.js"]} wide>
      <article class="clip">
        <div class="player" data-player>
          <video
            src={urls.video}
            poster={urls.thumb ?? undefined}
            controls
            playsinline
            preload="metadata"
            width={clip.width}
            height={clip.height}
          />
          <div class="vignette" />
          <button type="button" class="play lg lg-refract" aria-label={t.play} data-play>
            <svg width="30" height="30" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
              <path d="M8 5.5v13a1 1 0 0 0 1.5.9l10.4-6.5a1 1 0 0 0 0-1.7L9.5 4.6A1 1 0 0 0 8 5.5z" />
            </svg>
          </button>
          <span class="chip lg">{formatDuration(clip.duration_ms)}</span>
        </div>
        <div class="clip-bar">
          <div class="clip-info">
            <h1>{clip.title}</h1>
            <p class="byline">
              <span class="avatar" aria-hidden="true">
                {initial}
              </span>
              <span>
                <strong>{props.owner}</strong>
                {clip.game && <> · {clip.game}</>} · {formatDate(clip.created_at)}
                {clip.visibility !== "unlisted" && <> · {t.visibility[clip.visibility]}</>}
              </span>
            </p>
          </div>
          <div class="clip-actions">
            {!isPrivate && (
              <button class="btn lg-size" type="button" data-copy={urls.page} data-copied={t.copied}>
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true">
                  <path d="M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1" />
                </svg>
                <span data-label>{t.copyLink}</span>
              </button>
            )}
            <a
              class="btn ghost round lg-size lg"
              href={`${urls.video}${urls.video.includes("?") ? "&" : "?"}download=1`}
              aria-label={t.download}
              title={t.download}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M12 4v11M7 10l5 5 5-5M5 20h14" />
              </svg>
            </a>
          </div>
        </div>
      </article>
    </Layout>
  );
}
