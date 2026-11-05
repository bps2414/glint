/** @jsxImportSource hono/jsx */
import type { User } from "../env";
import { t } from "../i18n";
import { Layout } from "./layout";

/** Envio pelo navegador (teste da Fase 4). Usa a mesma API do app — ver /static/enviar.js. */
export function UploadPage(props: { user: User }) {
  return (
    <Layout title={t.uploadTest} user={props.user} scripts={["enviar.js"]}>
      <section class="card narrow">
        <h1>{t.uploadTest}</h1>
        <p class="muted">{t.uploadIntro}</p>
        <form id="upload" class="stack">
          <label>
            {t.uploadFile}
            <input name="file" type="file" accept="video/mp4" required />
          </label>
          <label>
            {t.uploadTitle}
            <input name="title" maxlength={100} />
          </label>
          <label>
            {t.uploadGame}
            <input name="game" maxlength={60} />
          </label>
          <label>
            {t.uploadVisibility}
            <select name="visibility">
              <option value="unlisted" selected>
                {t.visibility.unlisted}
              </option>
              <option value="public">{t.visibility.public}</option>
              <option value="private">{t.visibility.private}</option>
            </select>
          </label>
          <button class="btn" type="submit">
            {t.uploadSubmit}
          </button>
          <progress id="progress" max={100} value={0} hidden />
          <p id="status" class="muted" role="status" />
        </form>
      </section>
    </Layout>
  );
}
