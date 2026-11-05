/** @jsxImportSource hono/jsx */
import { t } from "../i18n";
import { Layout } from "./layout";

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
