# Plano 1.1.0: onboarding novo + launcher

Decisões (2026-11-10):
- **Quando:** depois do `PLANO-feedback.md`. Sai na 1.1.0, junto com o onboarding de servidor, o feedback e os fixes.
- **Execução:** tudo de uma vez, numa entrega só, sem fases. A ordem abaixo é só a sequência de trabalho; nada é commitado ou publicado pela metade.
- **Onboarding refeito:** quem vem de qualquer versão anterior (1.0.x e RCs) passa pelo onboarding novo uma vez. Instalação nova também.
- **Conta mantida:** ninguém é deslogado. O passo do servidor mostra "Conectado como X" com opção de trocar.
- **Launcher:** splash no próprio `glint.exe` (sem exe novo, sem pastas por versão). Aparece na abertura fria (janela e gravador fechados).
- **Gravador também atualiza:** como ele sobe com o Windows e fica na bandeja, ele checa e instala sozinho em segundo plano quando está ocioso.

## Gatilho do onboarding

- `Config::onboarding: u32` substitui `welcome_done`. Constante `ONBOARDING_ATUAL = 2` no app.
- Config antiga não tem o campo → 0 → mostra. Concluir ou pular grava 2. Subir a constante no futuro refaz o onboarding de novo.
- `welcome_done` continua sendo lido (`#[serde(default, skip_serializing)]`) só pra saber se é quem atualizou (texto da boas-vindas muda).
- Some o `welcome.rs` e as chaves `welcome.*`; `views/server.rs:259` passa a usar o campo novo.

## Onboarding (`views/onboarding.rs`)

Tela cheia dentro da janela (não modal): sem sidebar, cartão central, indicador de 6 passos, Voltar / Continuar e "Pular configuração" em todos (pular grava os valores atuais ou os recomendados e marca concluído). Cada passo grava na hora com `apply` (que já manda `ReloadConfig` pro gravador).

1. **Boas-vindas.** Instalação nova: o que o Glint faz em uma frase. Quem atualizou: "O Glint ficou mais simples" + 3 novidades (entrar por convite, criar servidor pelo app, reportar bug).
2. **Teclas.** Duas caixas "clique e aperte a tecla" (salvar e kill), componente novo `components/key_capture.rs`. Conflito entre as duas (`same_hotkey`) e tecla ocupada por outro app (`hotkey_ok`/`kill_hotkey_ok` do status) aparecem na hora. Teste: "Aperte F8 agora" → check quando o gravador conta o aperto.
   - IPC: `RecorderStatus` ganha `save_presses` e `kill_presses` (contam todo aperto, gravando ou não); sobe a versão do protocolo. Neste passo a UI consulta o status a cada 250 ms.
   - Gravador fechado: botão "Iniciar gravador" no lugar do teste.
3. **Qualidade.** Mostra a GPU (nome do adaptador DXGI; o plano de feedback já cria isso no `gpu.rs`) e testa os encoders em segundo plano (`app_media::video::available_encoders()`: tenta abrir cada candidato de `hw_candidates` em 1080p).
   - Leve: H.264, 720p, 60 fps, 12 Mbps. Equilibrado: HEVC, 1080p, 60 fps, 25 Mbps (o padrão de hoje). Máximo: HEVC, nativa, 60 fps, 40 Mbps.
   - Recomendado: HEVC por hardware → Equilibrado; só H.264 por hardware → Equilibrado com H.264; nenhum → Leve (software). Máximo nunca é marcado sozinho.
   - Quem atualizou: vem marcado o que já está no config ("Personalizado" se não bate com nenhum); nada muda sem a pessoa escolher.
   - Áudio: chaves "som do jogo" e "microfone", com medidor de nível do microfone (captura WASAPI curta na UI só enquanto o passo está aberto, via `app_platform_win::audio::spawn_capture`).
4. **Servidor.**
   - Conectado: "Conectado em `<servidor>` como `<nome>`" + Continuar / Trocar de servidor (sai e cai no estado abaixo, com confirmação).
   - Sem servidor: campo pra colar o link de convite direto no passo (mesma lógica do `JoinView`: nome = usuário do Windows), "Tenho o código de outro PC", "Criar meu servidor" (abre o assistente; ao terminar volta pro passo seguinte) e "Agora não".
5. **Sistema.** Três chaves: iniciar com o Windows (`start_with_os`), mostrar no Discord (`discord_presence`), atualizar sozinho (`check_updates`).
6. **Pronto.** Resumo (teclas, qualidade, servidor) + "Começar" → grava `onboarding = 2` e mostra a biblioteca.

Textos em `en.yml` e `pt-BR.yml`.

## Launcher (splash no `glint.exe`)

Quando aparece (decidido no `main.rs`, depois do `instance::claim`):
- Janela já aberta → como hoje (passa o link e fecha).
- Gravador não responde no IPC (abertura fria) ou veio `--from-installer` → splash.
- Senão (gravador na bandeja) → janela principal direto.
- Link `glint://` numa abertura fria passa pelo splash e é entregue à janela no fim.

Janela: gpui, ~320×380, sem barra de título, não redimensionável, centralizada, aparece na barra de tarefas. Logo, uma linha de status e barra de progresso. Fechar o splash cancela tudo (inclusive download). Sequência:

1. **"Procurando atualizações"**: `latest_release` com timeout de 5 s (variante nova em `update.rs`). Pula se `check_updates` desligado ou se veio `--from-installer`. Erro ou offline → segue.
2. **Versão nova:** "Baixando 1.1.1 (42%)" → SHA-256 → "Instalando" → abre o instalador (mesmo `run_installer` do `updater.rs`, `/SILENT`, com a barrinha do Inno) e sai. O instalador reabre com `glint.exe --from-installer`. Falha → "Não deu pra atualizar" por 2 s e segue na versão atual (o card da sidebar continua oferecendo).
3. **"Iniciando gravador"**: `recorder::start()` e espera o `Hello` no IPC (até 10 s; falhou → segue, a barra superior já mostra "Gravador fechado" com botão).
4. **"Carregando biblioteca"**: config, `Store::open`, miniaturas da primeira tela e `fetch_account`/`server_info` (rede com timeout de 3 s, não bloqueia).
5. Abre a janela principal com isso pronto (`Shell::new` recebe um `Preloaded`) e fecha o splash. Sem tempo mínimo artificial.

## Gravador atualiza em segundo plano

- `app-recorder` passa a depender de `app-cloud` (só `update.rs`).
- 60 s depois de subir e a cada 6 h, se `check_updates`: `latest_release`; versão nova → baixa pro temp (SHA-256) numa thread de baixa prioridade.
- Instala só ocioso: nenhuma sessão gravando, nenhum jogo detectado, janela do app fechada (socket da UI não responde). Senão tenta de novo a cada 10 min.
- Roda o instalador com `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /glintbg`. O instalador fecha o gravador (`--quit`, como hoje), instala e reabre só o gravador.

## Instalador (`installer/glint.iss`)

- `[Code]`: `IsBackground` = `/glintbg` na linha de comando.
- `[Run]` do `glint.exe`: `Parameters: "--from-installer"` e `Check: not IsBackground`. O do gravador continua igual.

## Teste de atualização

- Variável `GLINT_UPDATE_FEED`: URL de um `latest.json` no lugar do GitHub (launcher e gravador). Só pra testar.
- Teste: build com versão falsa menor (`1.0.9`) instalado; `dist/` servido com `uv run python -m http.server`; `latest.json` local apontando pro instalador 1.1.0.

## Versão e docs

- Workspace `1.1.0` (o `server.json` do release segue junto, como hoje).
- CHANGELOG 1.1.0: onboarding novo, launcher, atualização em segundo plano, mais o que vem dos planos de servidor e feedback.
- READMEs (EN e pt-BR): primeira abertura e como o app se atualiza (e como desligar).
- A tag `v1.1.0` fica comigo.

## Ordem de trabalho e pronto

1. Base: `Config::onboarding`, contadores no IPC (+ versão do protocolo), `available_encoders`, timeout no `update.rs`, `GLINT_UPDATE_FEED`.
2. Onboarding (substitui o welcome).
3. Launcher + `Preloaded` no `Shell`.
4. Atualização pelo gravador + instalador.
5. Versão, CHANGELOG, READMEs.
6. Verificação:
   - `cargo build --release` e `cargo test` dos crates mexidos.
   - Config de 1.0.x (`welcome_done = true`, token salvo): onboarding completo aparece, passo do servidor mostra "Conectado como X", qualidade vem como estava, e depois de concluir não volta.
   - Instalação limpa: onboarding aparece; "Pular configuração" grava os recomendados.
   - F8 e F9 marcam o check no passo das teclas; tecla ocupada por outro app aparece.
   - Tudo fechado → splash → biblioteca já com miniaturas. Gravador na bandeja → abre direto. Offline → splash passa em até ~5 s.
   - Launcher atualiza do 1.0.9 falso pro 1.1.0 e reabre com `--from-installer`.
   - Gravador atualiza com o app fechado e sem jogo, sem abrir janela; com o app aberto ou gravando, espera.
7. Um commit só com tudo.

## Riscos

- Instalação silenciosa no meio de algo: só com tudo ocioso, e `check_updates` desliga.
- Abertura fria mais lenta: checagem com timeout curto, offline não trava.
- Splash no mesmo exe: se uma versão nova quebrar ao abrir, o launcher não salva (aceito ao escolher; saída é baixar o instalador anterior).
- Onboarding obrigatório irritar quem já configurou: valores atuais preservados e "Pular configuração" em todo passo.
- O instalador mata o `glint.exe` (`taskkill`): por isso o gravador só instala com a janela fechada.
