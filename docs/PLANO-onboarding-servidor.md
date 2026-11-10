# Plano: borda amarela no Win10 + onboarding de servidor

Decisões (2026-11-10):
- **Login:** o convite vira a conta. Entrar = clicar no link de convite; o app recebe um token do aparelho. Discord fica opcional (só se o dono configurar o app do Discord).
- **Criar servidor:** assistente dentro do app, via API REST do Cloudflare (sem wrangler, sem GitHub).

Ordem: Fase 1 é independente (sai sozinha num rc). A 2 vem antes da 3; a 4 depende da 2 e da 3.

## Fase 1: borda amarela no Windows 10

Causa: a captura usa WGC; `SetIsBorderRequired` só existe a partir do build 20348 (Win11). No Win10 (19045) a chamada falha e o erro só vai pro log (`crates/app-platform-win/src/capture.rs`).

- `capture_dxgi.rs` em `app-platform-win`: DXGI Desktop Duplication (`IDXGIOutput5::DuplicateOutput1`, BGRA). Sem borda, sem injeção (seguro com Vanguard). Escreve no mesmo `CaptureShared.latest`; encoder e sessão não mudam.
- Modo janela: recorta o monitor no retângulo da janela (`DWMWA_EXTENDED_FRAME_BOUNDS`) a cada frame.
- `DXGI_ERROR_ACCESS_LOST` (troca de resolução, UAC, tela cheia exclusiva): recria a duplicação.
- Notebook híbrido: duplica na GPU do monitor e copia pra do encoder. Se a duplicação falhar, volta pro WGC (com borda) e registra no log.
- `Capture::start` decide: propriedade `IsBorderRequired` presente → WGC; senão → DXGI. `capture_backend = auto|wgc|dxgi` no config.toml pra diagnóstico.
- Verificação: máquina Win10 real, jogo em tela cheia e em janela.

## Fase 2: servidor (convite vira conta)

- Migração `0002`: `users.discord_id` opcional (rebuild da tabela no SQLite) e campo `name`. Contas e sessões existentes continuam valendo.
- `GET /api/v1/info`: nome, versão do servidor e da API (compatibilidade com o app).
- `POST /api/v1/join {invite, name}`: gasta o convite, cria a conta, devolve token do aparelho. Rate limit.
- `POST /api/v1/invites`, `GET /api/v1/members` (+ desativar/revogar aparelho): admin gerencia pelo app.
- `POST /api/v1/setup {setup_token}`: cria o admin do servidor novo (substitui "primeiro login vira admin").
- `POST /api/v1/devices/link`: código curto pra entrar com o mesmo usuário em outro PC.
- Página `/i/<código>`: nome do servidor, quem convidou, **Abrir no Glint** (`glint://join?s=<origem>&i=<código>`) e **Baixar Glint**; "instale e clique no link de novo".
- Login no site (clipe privado, admin): o app abre o navegador já logado com um código de uso único.
- Discord: login só aparece se `DISCORD_CLIENT_ID` estiver configurado.
- Convites: uso único e validade por padrão.

## Fase 3: app (entrar num servidor)

- Protocolo `glint://` registrado pelo instalador (`HKCU\Software\Classes\glint`); app já aberto recebe o link via `instance.rs`.
- Janela "Entrar num servidor": aceita o link de convite colado (o servidor sai do link) ou chega preenchida pelo `glint://`. Nome preenchido com o usuário do Windows. Botão Entrar.
- Configurações → Servidor:
  - sem servidor: Entrar num servidor / Criar meu servidor;
  - conectado: nome do servidor, "você é X", Convidar amigo (copia link), Membros (admin), Conectar outro PC, Sair, Atualizar servidor (se desatualizado).
- Welcome: 4º passo "Links pro Discord" (Entrar / Criar / Pular). "Connect server" do Publicar abre a mesma janela.

## Fase 4: assistente "Criar meu servidor"

- O app abre o link de criação de token do Cloudflare com as permissões pré-preenchidas (Workers, D1, R2); usuário cola o token uma vez; fica no cofre do Windows (`credentials.rs`).
- Via API REST: cria D1 + migrações, bucket R2, sobe o Worker empacotado no release do app, assets, cron, subdomínio workers.dev, gera `SIGNING_KEY` e `SETUP_TOKEN`, chama `/api/v1/setup`; app sai conectado como admin.
- Atualizar servidor usa o mesmo caminho (Worker vai em cada versão do app).
- Aviso: o R2 exige cartão cadastrado no Cloudflare, mesmo no plano grátis; o assistente avisa e manda pra página certa.

## Fase 5: site e docs

- Seção `#server` do site (EN e PT): "Crie pelo app" e "Recebeu um convite? Instale e clique no link".
- `docs/DEPLOY.md`: assistente como caminho principal; deploy manual como avançado, Discord opcional.
- READMEs (EN e pt-BR) e CHANGELOG.

## Riscos

- Link de convite é a credencial de entrada (uso único, validade, rate limit).
- Token do aparelho = conta; admin revoga aparelhos em Membros.
- Sobra um passo manual na criação: gerar e colar o token do Cloudflare.
