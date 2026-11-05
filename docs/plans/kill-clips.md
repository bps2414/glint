# Plano: clipe de kill

Hotkey separada (padrão **F9**) que salva um clipe curto em volta da jogada, em vez dos 60 s do F8.
Kills em sequência viram um clipe só. Detecção automática fica fora desta versão (ver "Depois").

## Comportamento

- **Aperto simples:** clipe de `pre_secs` antes do aperto até `post_secs` depois (padrão 8 s + 2 s ≈ 10 s).
  O salvamento espera o pós-roll terminar.
- **Combo:** cada aperto abre uma janela de `combo_secs` (padrão 6 s). Novo aperto dentro dela estende o
  mesmo clipe: vai de `primeiro_aperto - pre` até `último_aperto + post`. Salva quando a janela fecha sem
  novo aperto. Ex.: kills com 4 s de intervalo → um clipe de ~14 s marcado "x2".
- **Teto:** se o combo passar do tamanho do buffer (`buffer_secs`), o clipe atual é salvo e o aperto
  começa outro.
- **Som:** cada aperto toca um "tick" curto (diferente do blip do F8), pra confirmar no meio da troca.
  Não toca nada ao salvar (o salvamento acontece até 6 s depois e confundiria); falha toca o blip de erro.
- **F8 continua igual** e independente: apertar F8 durante um combo salva os 60 s normalmente.
- **Jogo fechou / gravação parou com combo pendente:** salva na hora com o que tiver no buffer, antes de
  soltar a sessão.
- **Sem gravação ativa:** blip de erro, igual ao F8.
- Clipe vai pra biblioteca como tipo **Kill**, com contagem de apertos, filtro próprio e nome
  `Glint_<JOGO>_Kill_<data>.mp4`. Entra no mesmo limite de disco.

Precisão: o vídeo começa no keyframe anterior ao início pedido (keyframe a cada 1 s), então o clipe pode
ter até ~1 s a mais no começo. O fim é cortado exato (não precisa de keyframe). Continua sendo remux, sem
re-encode.

## Config (`[kill]` no `config.toml`)

```toml
[kill]
hotkey = "F9"
pre_secs = 8      # 3-30
post_secs = 2     # 0-10
combo_secs = 6    # 0-15 (0 = sem combo)
```

`save_sound` global continua valendo para o tick.

## Mudanças por crate

### app-core
- `config.rs`: `KillConfig` com os campos acima + `Default`; campo `kill` em `Config` (`#[serde(default)]`,
  config antiga carrega com os padrões). Clamp dos valores ao carregar.
- `replay.rs`: `ReplayBuffer::snapshot_range(start_us, end_us)`: vídeo do keyframe `<= start_us` até
  `<= end_us`, áudio no mesmo intervalo. Os timestamps já são QPC em µs (`app_platform_win::clock::now_us`),
  então o instante do aperto mapeia direto pros pacotes.
- `library.rs`: `SCHEMA_VERSION = 2`, migração com
  `ALTER TABLE clips ADD COLUMN kind TEXT NOT NULL DEFAULT 'replay'` e `ADD COLUMN kills INTEGER`.
  `NewClip`/`Clip` ganham `kind: ClipKind { Replay, Kill }` e `kills: Option<u32>`; `CLIP_COLUMNS` e
  `clip_from_row` atualizados.
- `naming.rs`: `clip_stem` recebe o tipo (ou um sufixo) pra gerar `..._Kill_<data>`.

### app-platform-win
- `hotkey.rs`: hoje, se **uma** tecla falha no `RegisterHotKey`, o listener inteiro falha. Mudar pra
  registrar cada tecla de forma independente e devolver quais funcionaram, pra F9 ocupado não derrubar o F8.
- `sound.rs`: `play_kill_tick()` com um WAV curto e mais agudo que o blip.

### app-recorder
- `daemon.rs`:
  - Registrar `[save_hotkey, kill.hotkey]`; `Event::Hotkey(idx)` → `save_clip()` ou `kill_press()`.
    `hotkey_ok` vira um por tecla.
  - Estado `pending_kill: Option<PendingKill { first_us, last_us, presses, deadline: Instant }>`.
  - `kill_press()`: sem sessão → erro. Com pendente e dentro da janela e sem estourar o buffer → estende;
    senão → `flush_kill()` do anterior e abre outro. Toca o tick.
  - Loop `run`: o `recv_timeout` passa a usar o menor entre `next_tick` e `deadline` do pendente
    (o `TICK` de 1 s atrasaria o salvamento). Venceu → `flush_kill()`.
  - `flush_kill()`: `session.snapshot_range(first_us - pre, last_us + post)` → `SaveJob` com
    `kind = Kill`, `kills = presses`, `play_sound = false`.
  - Antes de qualquer caminho que solta a sessão (`reconcile`, saída do `run`): `flush_kill()`.
  - `reload_config`: re-registrar se qualquer das duas hotkeys mudou.
- `session.rs`: `snapshot_range` repassando pro buffer.
- `saver.rs`: `SaveJob`/`ClipMeta` com `kind`, `kills`; nome via `naming`; miniatura no instante do
  primeiro aperto (≈ `pre_secs` dentro do clipe) em vez do ponto padrão.
- Bandeja: item "Salvar kill (F9)" e aviso de tecla ocupada por hotkey.
- Locales `en.yml`/`pt-BR.yml`.

### app-ipc
- `Request::SaveKill` (a UI/bandeja podem disparar), `RecorderStatus` com `kill_hotkey` e o estado de cada
  hotkey. Mudança de struct quebra o postcard: `PROTOCOL_VERSION = 2`.

### app-ui
- Configurações: seção "Clipe de kill" com hotkey + pré/pós/combo; validar que não é igual ao F8.
- Biblioteca: item "Kills" na navegação (`Filter::Kills` em `data.rs`) e selo no card ("Kill", "x2", "x3"...).
- Locales `en.yml`/`pt-BR.yml`.

### Docs
- `README.md`/`README.pt-BR.md` (hotkeys), `ARCHITECTURE.md` (seção do buffer), `CHANGELOG.md`.

## Ordem

1. app-core (config, `snapshot_range`, migração, naming) com testes unitários do range e da migração
   (já existe padrão de teste nesses arquivos).
2. hotkey independente + tick.
3. daemon/saver (máquina de estado do combo).
4. IPC + UI.
5. Docs.

## Verificação

- `cargo test -p app-core` (range, migração v1→v2, config antiga).
- `cargo build` do workspace.
- No jogo: F9 uma vez (~10 s, "Kill"); F9 duas vezes com 4 s de intervalo (um clipe ~14 s, "x2");
  F9 duas vezes com 8 s (dois clipes); F8 no meio de um combo (os dois saem); fechar o jogo com combo
  pendente (salva); F9 com outro app usando a tecla (F8 continua funcionando, bandeja avisa).

## Depois (fora desta versão)

- **Detecção automática** onde há API oficial: CS2 (Game State Integration) e LoL (Live Client Data API,
  `127.0.0.1:2999`). Valorant não tem API de kill; só daria por áudio/visão, o que custa CPU durante o jogo.
- **Botão lateral do mouse** como hotkey (precisa de hook `WH_MOUSE_LL`).
- **Montagem** das kills da sessão num vídeo só.
