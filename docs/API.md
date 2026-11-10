# API do servidor (v1)

Contrato entre o app (Fase 6) e o Worker (`server/`). Base: `https://<worker>.workers.dev`.
Os tipos aqui são a fonte da verdade. Mudança incompatível = `/api/v2`.

## Autenticação

- **App:** `Authorization: Bearer <token>` (token de 180 dias, guardado no Credential Manager).
- **Navegador:** cookie `__Host-sess` (30 dias). Requisição que muda estado e leva cookie precisa ter
  `Origin` deste site (proteção CSRF). Não existe CORS.
- Token de app não vale como cookie, e o contrário também não.

### Login do app (loopback + PKCE, RFC 8252)

1. O app gera `verifier` (32 bytes aleatórios, base64url sem `=`, 43 caracteres) e
   `challenge = base64url(SHA-256(verifier))`. Também gera um `state` aleatório (8 a 128 caracteres `[A-Za-z0-9_-]`).
2. O app escuta em `127.0.0.1:<porta>` (porta livre, 1024–65535) e abre no navegador:
   `GET /auth/app?port=<porta>&challenge=<challenge>&state=<state>`
3. O usuário entra com o Discord. Se a conta é nova, o convite é pedido no navegador.
4. O navegador é redirecionado para `http://127.0.0.1:<porta>/callback?code=<code>&state=<state>`.
   O app confere o `state` e responde com uma página simples ("pode fechar esta aba").
5. `POST /api/v1/auth/token` com `{"code": "...", "verifier": "..."}` → `{"token": "...", "expires_at": <ms>}`.
   O código vale **2 minutos** e é usado **uma vez só** (uma tentativa errada já o consome).
6. Sair: `POST /api/v1/auth/logout` (Bearer) → `204`.

Esse login só existe com Discord configurado (`GET /api/v1/info` → `discord_login`). O caminho
principal é o convite virar a conta, abaixo.

### Convite vira conta (sem Discord)

Todas devolvem `201 { "token": "...", "expires_at": <ms>, "user": User }` (token de app, 180 dias).

- `POST /api/v1/join` `{"invite": "<código ou link .../i/<código>>", "name": "Bryan"}`: gasta o convite e
  cria a conta. `400 invalid_invite` se o convite é inválido, vencido, revogado ou já usado. Nome: 1–32
  caracteres, sem caracteres de controle. 10 tentativas por IP a cada 10 min.
- `POST /api/v1/setup` `{"setup_token": "...", "name": "..."}`: cria o admin de um servidor **sem
  usuários**. Só existe com o segredo `SETUP_TOKEN` (16+ caracteres) configurado (senão `404`); token
  errado `403`; servidor que já tem usuário `409 already_setup`. Com `SETUP_TOKEN`, o primeiro login
  Discord deixa de virar admin.
- Outro PC: no PC conectado, `POST /api/v1/devices/link` (Bearer) → `201 {"code": "K7QF-2M9X", "expires_at"}`
  (10 minutos, uso único). No PC novo, `POST /api/v1/devices/link/claim` `{"code": "k7qf2m9x"}` (maiúsculas,
  hífen e espaços tanto faz) → token do mesmo usuário. Código errado ou vencido: `400 invalid_code`.

### Abrir o site já logado

`POST /api/v1/auth/web-code` (Bearer) `{"next": "/admin"}` → `{"url": "https://.../auth/web?code=...", "expires_at"}`.
O app abre `url` no navegador; vale **1 minuto**, uma vez só, e vira sessão web (cookie). `next` só aceita
caminho relativo (senão vai para `/`).

## Erros

Sempre neste formato, com `message` em PT-BR pronta para mostrar:

```json
{ "error": { "code": "quota_exceeded", "message": "Sua cota ...", "used_bytes": 123, "quota_bytes": 456 } }
```

| HTTP | `code` | Quando |
|---|---|---|
| 400 | `invalid` | Corpo/parâmetros inválidos |
| 400 | `part_size` | Parte com tamanho diferente do esperado (`expected_bytes`) |
| 400 | `bad_thumb` | Miniatura não é JPEG ou passa de 1 MB |
| 400 | `invalid_code` | Código de login inválido, vencido ou verifier errado |
| 400 | `invalid_invite` | Convite inválido, vencido, revogado ou já usado (`join`) |
| 401 | `unauthorized` | Sem token/sessão válida → refazer login |
| 403 | `forbidden` | Não é admin, ou `setup_token` errado |
| 409 | `already_setup` | `setup` num servidor que já tem usuário |
| 409 | `self` | Admin tentando desativar a própria conta |
| 404 | `not_found` | Não existe **ou é de outra pessoa** |
| 409 | `quota_exceeded` | Passaria da cota do usuário (só com `USER_QUOTA_GB` > 0) (`used_bytes`, `quota_bytes`) |
| 409 | `missing_parts` | `complete` antes de todas as partes (`parts_done`, `part_count`) |
| 409 | `not_uploading` | Parte enviada para clipe já concluído |
| 411 | `length_required` | Parte sem `Content-Length` |
| 413 | `too_large` | Vídeo maior que o limite (`max_clip_bytes`) |
| 422 | `bad_video` | Arquivo final não é MP4 ou o tamanho não confere (o clipe é apagado) |
| 429 | `rate_limited` | Muitas requisições |
| 500 | `server` | Erro interno |
| 507 | `storage_full` | O servidor inteiro passaria de `STORAGE_LIMIT_GB` (`used_bytes`, `limit_bytes`); não adianta tentar de novo |

## Tipos

```ts
type Visibility = "private" | "unlisted" | "public"; // padrão: "unlisted" (com link)

interface Clip {
  id: string;              // 22 caracteres base64url (128 bits)
  title: string;
  game: string | null;
  visibility: Visibility;
  status: "uploading" | "ready";
  size_bytes: number;      // tamanho do MP4
  duration_ms: number;
  width: number;
  height: number;
  created_at: number;      // ms desde a época
  url: string | null;      // página do clipe (para copiar/colar no Discord); null enquanto "uploading"
  video_url: string | null;// MP4 direto; privado = URL assinada que vence em 1 h
  thumb_url: string | null;
}

interface UploadState {
  part_size: number;       // hoje 8 MiB (8388608)
  part_count: number;
  parts_done: number[];    // partes já recebidas (para retomar)
}
```

## Rotas

### `GET /api/v1/info` (público)

```json
{ "name": "Glint", "server_version": "1.1.0", "api_version": 1, "discord_login": false, "setup_required": false }
```

`name` = `SERVER_NAME` (ou o nome do app). `setup_required`: servidor sem usuários esperando o `POST /setup`.

### `GET /api/v1/me`

`User`: `name` é o nome mostrado no servidor. `username` continua por compatibilidade (conta por convite:
igual a `name`); `display_name` e `avatar_url` só vêm do Discord (`null` sem Discord).

```json
{
  "user": { "id": "...", "name": "...", "username": "...", "display_name": "...", "avatar_url": "https://cdn.discordapp.com/...", "role": "user" },
  "quota": { "used_bytes": 0, "quota_bytes": 21474836480 },  // quota_bytes 0 = sem cota por usuário
  "limits": { "max_clip_bytes": 524288000, "part_size": 8388608, "max_thumb_bytes": 1048576 }
}
```

### Publicar um clipe

1. `POST /api/v1/clips`
   ```json
   { "title": "Ace", "game": "VALORANT", "visibility": "unlisted",
     "size_bytes": 12345678, "duration_ms": 30000, "width": 1920, "height": 1080 }
   ```
   → `201 { "clip": Clip, "upload": UploadState }`. Reserva `size_bytes` na cota.
   Upload não concluído em **24 h** é apagado e libera a cota.
2. Para cada parte `n` de `1..part_count`: `PUT /api/v1/clips/<id>/parts/<n>`, corpo = bytes
   `[(n-1)*part_size, n*part_size)` do arquivo, `Content-Length` obrigatório e exato (a última é menor).
   → `200 { "part_number": n }`. Reenviar a mesma parte substitui a anterior. Ordem livre.
3. (Opcional) `PUT /api/v1/clips/<id>/thumb`, `Content-Type: image/jpeg`, até 1 MB → `204`.
   É a imagem do embed. Pode ser trocada depois.
4. `POST /api/v1/clips/<id>/complete` → `200 { "clip": Clip }` (agora `ready`, com `url`).
   O servidor confere o tamanho exato e a assinatura MP4 (`ftyp`). Chamar de novo é seguro.

**Retomar:** `GET /api/v1/clips/<id>` → `{ "clip": Clip, "upload": UploadState | null }` e enviar só o que
falta em `parts_done`.

O vídeo publicado deve ser **H.264 + AAC** (uma trilha de áudio mixada) com `+faststart` (moov no início),
para tocar no navegador e no embed do Discord.

### Gerenciar

- `GET /api/v1/clips?before=<created_at>` → `{ "clips": Clip[], "next_before": number | null }` (50 por página, mais novo primeiro).
- `GET /api/v1/clips/<id>` → `{ "clip": Clip, "upload": UploadState | null }`.
- `PATCH /api/v1/clips/<id>` com `{ "title"?: string, "visibility"?: Visibility }` → `{ "clip": Clip }`.
- `DELETE /api/v1/clips/<id>` → `204` (apaga vídeo e miniatura do R2 e libera a cota).

### Admin (role `admin`; outro usuário recebe `403 forbidden`)

- `POST /api/v1/invites` `{"note"?: string, "max_uses"?: 1..100 (1), "days"?: 0..365 (7; 0 = sem prazo)}` →
  `201 {"invite": {"id", "url", "note", "max_uses", "expires_at"}}`. O `url` (`.../i/<código>`) só aparece aqui.
- `GET /api/v1/members` → `{"members": [User + {"discord": bool, "disabled": bool, "created_at", "last_login_at",
  "used_bytes", "clips", "devices": [{"id", "kind": "app"|"web", "created_at", "last_used_at", "expires_at", "current": bool}]}]}`.
- `POST /api/v1/members/<id>/disable` / `enable` → `204`. Desativar derruba todas as sessões. Na própria conta: `409 self`.
- `DELETE /api/v1/members/<id>/devices/<device>` → `204` (revoga aquele aparelho/sessão).

## Rotas públicas (sem API)

| Rota | O quê |
|---|---|
| `GET /i/<código>` | Página do convite: nome do servidor, quem convidou, **Abrir no Glint** (`glint://join?s=<origem>&i=<código>`) e **Baixar**. Não gasta o convite. Com Discord, link para `/i/<código>/discord` (guarda o código e manda para o login) |
| `GET /auth/web?code=` | Entrada no site pelo app (ver "Abrir o site já logado") |
| `GET /c/<id>` | Página do clipe com player e meta tags OpenGraph/Twitter (`og:video` = MP4) para o embed. Privado: só o dono vê (os outros recebem 404) e não há meta tags |
| `GET/HEAD /v/<id>.mp4` | Vídeo com suporte a `Range` (206/416). Privado exige `?exp&sig` (vem no `video_url`). `?download=1` baixa |
| `GET /t/<id>.jpg` | Miniatura (mesmas regras) |
| `GET /entrar`, `/convite`, `POST /sair` | Login web com Discord (sem Discord, `/entrar` explica que se entra pelo app) |
| `/admin` | Convites, usuários, cotas (só admin) |
| `/enviar` | Envio de teste pelo navegador (usa esta mesma API) |
