# Plano: feedback / reportar bug + Leave a star

Decisões (2026-11-10):
- **Destino:** um Worker central do mantenedor (não o servidor de cada grupo). Quem usa não precisa de conta no GitHub.
- **Star:** sem login no GitHub não dá pra dar star automático (a API exige o token da própria pessoa), e não vamos obrigar ninguém a criar conta. O botão abre a página do repo; quem tem conta clica em Star lá.
- **Execução:** tudo de uma vez, numa entrega só (Worker + app + docs), sem fases. A ordem abaixo é só a sequência de trabalho; nada é commitado ou publicado pela metade.

Independe das fases do onboarding de servidor; só encosta em `settings.rs` e nos locales, que a Fase 3 do onboarding também mexe (fazer depois dela, ou rebasear).

## Worker `feedback`

Fica em `server/feedback/` (como `server/redirect/`: `index.ts` + `wrangler.jsonc`, usa o `node_modules` do `server/`), na conta Cloudflare do Glint, em `feedback.getglint.workers.dev`.

- `POST /v1/report` (JSON): `kind` (`bug` | `feedback`), `message` (obrigatório, até 5 mil caracteres), `contact` (opcional), `app_version`, `os`, `gpu`, `capture_backend`, `locale` e `logs` (opcional, texto, até 512 KB).
- Validação de tamanho e campos; corpo acima do limite → 413.
- Antispam: binding de Rate Limiting do Workers por IP (ex.: 3/10 min) + header `X-Glint-Client: <versão>` (só filtra lixo, não é segredo).
- Guarda: D1 `reports` (id, kind, mensagem, contato, specs, criado_em, issue_url) e logs no R2 (`reports/<id>.log`), privados.
- Avisa: abre issue no `bps2414/glint` com label `bug`/`feedback` e `from-app`, com mensagem + specs, **sem logs e sem contato** (issue é pública). Token de um GitHub App instalado só no repo com `Issues: write` (ou PAT fine-grained), em `wrangler secret`. Se a issue falhar, o report já está salvo; responde 200 mesmo assim.
- `GET /v1/reports/<id>/logs` com `Authorization: Bearer <ADMIN_TOKEN>` pra eu baixar o log citado na issue (a issue mostra só o id).
- Resposta: `{ id }`. Testes em vitest como no `server/` (limites, rate limit, falha do GitHub não perde o report).

## App

- `app-cloud/src/feedback.rs`: `send_report(&Report) -> Result<String>` (ureq, timeouts como o `update.rs`). URL fixa em `brand.rs` (`FEEDBACK_URL`), igual ao `GITHUB_REPO`.
- Coleta de logs: as 2 últimas `recorder*.log` de `Config::data_dir()/logs`, cortadas no final pra caber em 512 KB.
  - Limpeza antes de enviar: caminho do perfil (`C:\Users\<nome>`) → `%USERPROFILE%`, tokens/`Bearer ...` e query strings de URLs assinadas removidos. Conferir se o `api.rs` loga algo sensível.
  - O app (`glint.exe`) hoje não grava log em arquivo, só `eprintln!`. Fora do escopo; os logs do gravador cobrem a captura, que é onde mora a maioria dos bugs.
- Specs: versão, build do Windows, GPU (`gpu.rs`), backend de captura, idioma.
- Janela (modal, `components/modal.rs`), `views/feedback.rs`:
  - Abas ou seletor: Reportar bug / Enviar feedback.
  - Campo de texto, contato opcional ("e-mail ou Discord, se quiser resposta").
  - "Incluir logs e informações do PC" (marcado em bug, desmarcado em feedback) + "Ver o que será enviado" (abre o texto final).
  - Enviar → estado de envio → "Recebido, obrigado" (com o id) ou erro com "Tentar de novo".
- Configurações → nova seção "Ajuda" (depois de Sistema): Reportar bug, Enviar feedback, Leave a star (abre `https://github.com/bps2414/glint` com `cx.open_url`).
- Menu da bandeja: "Reportar bug" abre o app direto na janela (via `instance.rs`, igual ao link `glint://`).
- Textos em `en.yml` e `pt-BR.yml`.

## Docs

- `docs/API.md` não (é do servidor); `SECURITY.md`: lembrar que falha de segurança não vai pelo botão, vai pelo advisory privado (a janela mostra isso no modo bug).
- README (EN e pt-BR) e CHANGELOG: botão de feedback e o que é enviado (privacidade: logs ficam privados, só a mensagem vira issue pública).

## Ordem de trabalho e pronto

1. Worker com testes passando (`vitest`) e deploy em `feedback.getglint.workers.dev` (D1, R2, rate limit e secrets criados).
2. App apontando pro Worker publicado: build ok e um report real de bug e um de feedback enviados do app, conferindo a linha no D1, o log no R2 e a issue criada sem logs/contato.
3. Docs.
4. Um commit só com tudo.

## Riscos

- Mensagem vira issue pública: a janela avisa "sua mensagem fica pública no GitHub; logs e contato não".
- Endpoint aberto atrai spam: rate limit por IP; se apertar, Turnstile não serve (app nativo), aí a saída é desligar a criação automática de issue e triar pelo D1.
- Log pode carregar dado pessoal mesmo limpo: opt-in visível e "ver o que será enviado".

## Status (2026-11-10)

Feito: Worker no ar em `feedback.getglint.workers.dev` (D1 `glint-feedback`, R2 `glint-feedback`, rate limit, `ADMIN_TOKEN` guardado em `D:\Bryan\Dev\Data\glint-feedback\admin-token.txt`, labels `bug`/`feedback`/`from-app` no repo) e testado em produção (report salvo, log no R2, download com token). App: envio, coleta e limpeza dos logs, janela, seção Ajuda, item "Reportar bug" na bandeja (`glint://feedback?kind=bug`), textos EN/pt-BR.

Falta:
- `GITHUB_TOKEN` (fine-grained, só `bps2414/glint`, Issues: Read and write): `npx wrangler secret put GITHUB_TOKEN -c feedback/wrangler.jsonc` em `server/`. Sem ele os reports são salvos mas nenhuma issue é aberta.
- Testar a janela de verdade: um bug e um feedback enviados pelo app, conferindo D1, R2 e a issue.
- Docs (SECURITY, READMEs, CHANGELOG).
