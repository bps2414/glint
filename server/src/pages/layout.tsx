/** @jsxImportSource hono/jsx */
import type { Context } from "hono";
import type { Child } from "hono/jsx";
import { APP_NAME, THEME_COLOR } from "../brand";
import type { AppEnv, User } from "../env";
import { lang, t } from "../i18n";

/** Login Discord ligado neste deploy (como a língua: um valor por deploy, setado a cada requisição). */
export let discordLogin = false;
export function setDiscordLogin(on: boolean) {
  discordLogin = on;
}

export function render(c: Context<AppEnv>, node: Child, status: 200 | 400 | 403 | 404 | 429 | 500 = 200) {
  c.header("Cache-Control", "no-store");
  return c.html(`<!doctype html>${String(node)}`, status);
}

export function Layout(props: {
  title?: string;
  head?: Child;
  user?: User | null;
  /** Scripts de /static (CSP só permite 'self'). */
  scripts?: string[];
  wide?: boolean;
  /** Sem o "Entrar" do cabeçalho (a página já tem o botão grande). */
  hideLogin?: boolean;
  children?: Child;
}) {
  const title = props.title ? `${props.title} · ${APP_NAME}` : APP_NAME;
  return (
    <html lang={lang}>
      <head>
        <meta charset="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="theme-color" content={THEME_COLOR} />
        <title>{title}</title>
        {/* `?v=2`: URL nova para quem guardou o site.css antigo (era cache de 1 dia). Agora o CSS
            revalida sempre (public/_headers), então não precisa mudar a cada versão. */}
        <link rel="stylesheet" href="/static/site.css?v=2" />
        <link rel="icon" href="/static/icon.svg" type="image/svg+xml" />
        <link rel="icon" href="/static/favicon.ico" sizes="16x16 32x32 48x48" />
        {props.head}
      </head>
      <body>
        <RefractFilter />
        <div class="ambient" aria-hidden="true" />
        <div class={props.wide ? "page" : "page narrow"}>
          <header class="top lg lg-refract">
            <a class="brand" href="/">
              <img src="/static/icon.svg" alt="" width={28} height={28} />
              <span class="wordmark">{APP_NAME.toLowerCase()}</span>
            </a>
            <div class="grow" />
            <nav class="top-nav">
              {props.user ? (
                <>
                  <a href="/">{t.myClips}</a>
                  {props.user.role === "admin" && <a href="/admin">{t.admin}</a>}
                  <a href="/enviar">{t.uploadTest}</a>
                  <form method="post" action="/sair" class="inline">
                    <button type="submit" class="link">
                      {t.logout}
                    </button>
                  </form>
                </>
              ) : props.hideLogin || !discordLogin ? null : (
                <a class="btn" href="/entrar">
                  {t.login}
                </a>
              )}
            </nav>
          </header>
          <main class="stack-main">{props.children}</main>
          <footer class="foot">
            <span>{t.madeWith(APP_NAME)}</span>
            <a href="/baixar">{t.downloadApp(APP_NAME)}</a>
          </footer>
        </div>
        {(props.scripts ?? []).map((s) => (
          <script src={`/static/${s}`} defer />
        ))}
      </body>
    </html>
  );
}

/**
 * Filtro de refração do vidro (mockup): ruído + deslocamento. Fica inline no HTML — é SVG,
 * não script nem estilo, e a CSP atual permite; o CSS só usa onde há suporte.
 */
function RefractFilter() {
  return (
    <svg class="svg-defs" width="0" height="0" aria-hidden="true">
      <filter id="glint-refract" x="0" y="0" width="100%" height="100%">
        <feTurbulence type="fractalNoise" baseFrequency="0.008 0.012" numOctaves="2" seed="7" result="n" />
        <feGaussianBlur in="n" stdDeviation="2" result="nb" />
        <feDisplacementMap in="SourceGraphic" in2="nb" scale="38" xChannelSelector="R" yChannelSelector="G" />
      </filter>
    </svg>
  );
}

export function Message(props: { title: string; text: string; user?: User | null; action?: Child }) {
  return (
    <Layout title={props.title} user={props.user}>
      <section class="card narrow">
        <h1>{props.title}</h1>
        <p class="muted">{props.text}</p>
        {props.action ?? (
          <a class="btn" href="/">
            {t.back}
          </a>
        )}
      </section>
    </Layout>
  );
}
