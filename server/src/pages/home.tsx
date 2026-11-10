/** @jsxImportSource hono/jsx */
import { APP_NAME } from "../brand";
import type { User } from "../env";
import { t } from "../i18n";
import type { ClipRow } from "../lib/clips";
import { formatBytes, formatDate, formatDuration } from "../lib/format";
import { discordLogin, Layout } from "./layout";

export function HomeLoggedOut(props: { notice?: string }) {
  return (
    <Layout hideLogin>
      <section class="hero">
        <h1 class="mark">
          <img src="/static/icon.svg" alt="" width={56} height={56} />
          <span class="wordmark">{APP_NAME.toLowerCase()}</span>
        </h1>
        <p class="muted">{t.tagline}</p>
        {props.notice && <p class="notice">{props.notice}</p>}
        {discordLogin ? (
          <a class="btn lg-size" href="/entrar">
            {t.login}
          </a>
        ) : (
          <p class="muted">{t.signInViaApp(APP_NAME)}</p>
        )}
      </section>
    </Layout>
  );
}

/** `quota` em bytes; 0 = servidor sem cota por usuário (mostra só o espaço usado). */
export function HomeLoggedIn(props: { user: User; used: number; quota: number; clips: ClipRow[] }) {
  const { user } = props;
  const pct = Math.min(100, (props.used / Math.max(1, props.quota)) * 100);
  return (
    <Layout user={user}>
      <section class="card">
        <h1>{t.hello(user.name)}</h1>
        <div class="meter-row">
          <span class="muted">{t.storage}</span>
          <span class="mono">
            {props.quota > 0 ? t.usage(formatBytes(props.used), formatBytes(props.quota)) : formatBytes(props.used)}
          </span>
        </div>
        {props.quota > 0 && <progress class="meter" max={100} value={pct.toFixed(1)} />}
      </section>
      <section class="card">
        <h2>{t.myClips}</h2>
        {props.clips.length === 0 ? (
          <p class="muted">{t.noClips}</p>
        ) : (
          <ul class="list">
            {props.clips.map((c) => (
              <li>
                <a href={`/c/${c.id}`}>{c.title}</a>
                <span class="muted">
                  {c.status === "uploading" ? t.uploading : t.visibility[c.visibility]} ·{" "}
                  <span class="mono">{formatDuration(c.duration_ms)}</span> · {formatBytes(c.size_bytes)} ·{" "}
                  {formatDate(c.created_at)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </Layout>
  );
}
