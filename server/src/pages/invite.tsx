/** @jsxImportSource hono/jsx */
import { APP_NAME } from "../brand";
import { t } from "../i18n";
import { Layout } from "./layout";

/** Página do link de convite (`/i/<código>`). */
export function InviteLanding(props: { server: string; inviter: string; openUrl: string; discordUrl?: string }) {
  return (
    <Layout title={t.inviteTitle} hideLogin>
      <section class="card narrow">
        <h1>{t.inviteJoinTitle(props.server)}</h1>
        <p class="muted">{t.inviteFrom(props.inviter, APP_NAME)}</p>
        <div class="stack">
          <a class="btn lg-size" href={props.openUrl}>
            {t.inviteOpenApp(APP_NAME)}
          </a>
          <a class="btn lg-size ghost" href="/baixar">
            {t.downloadApp(APP_NAME)}
          </a>
        </div>
        <p class="muted">{t.inviteInstallHint(APP_NAME)}</p>
        {props.discordUrl && (
          <p class="muted">
            <a href={props.discordUrl}>{t.inviteWithDiscord}</a>
          </p>
        )}
      </section>
    </Layout>
  );
}

export function InvitePage(props: { name: string; error?: string }) {
  return (
    <Layout title={t.inviteTitle}>
      <section class="card narrow">
        <h1>{t.inviteTitle}</h1>
        <p class="muted">{t.inviteIntro(props.name)}</p>
        {props.error && <p class="error">{props.error}</p>}
        <form method="post" action="/convite" class="stack">
          <label>
            {t.inviteCode}
            <input name="code" required autocomplete="off" spellcheck={false} maxlength={300} autofocus />
          </label>
          <button class="btn" type="submit">
            {t.inviteSubmit}
          </button>
        </form>
      </section>
    </Layout>
  );
}
