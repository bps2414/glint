-- Esquema inicial. Tempos em milissegundos desde a época (Date.now()).
-- IDs são aleatórios (128 bits em base64url); tokens e códigos só ficam como hash SHA-256.

CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  discord_id    TEXT NOT NULL UNIQUE,
  username      TEXT NOT NULL,
  display_name  TEXT,
  avatar        TEXT,
  role          TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin')),
  quota_bytes   INTEGER NOT NULL,
  disabled      INTEGER NOT NULL DEFAULT 0,
  invite_id     TEXT,
  created_at    INTEGER NOT NULL,
  last_login_at INTEGER NOT NULL
);

CREATE TABLE invites (
  id          TEXT PRIMARY KEY,
  code_hash   TEXT NOT NULL UNIQUE,
  -- Primeiros caracteres do código, só para o admin reconhecer na lista.
  hint        TEXT NOT NULL,
  note        TEXT,
  created_by  TEXT NOT NULL REFERENCES users(id),
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER,
  max_uses    INTEGER NOT NULL DEFAULT 1 CHECK (max_uses >= 1),
  uses        INTEGER NOT NULL DEFAULT 0,
  revoked_at  INTEGER
);

CREATE TABLE sessions (
  token_hash   TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL CHECK (kind IN ('web', 'app')),
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL,
  last_used_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE INDEX sessions_expires ON sessions(expires_at);

-- Etapas curtas do login: state do OAuth, cadastro esperando convite, código do app.
-- Cada linha é usada uma vez só (DELETE ... RETURNING).
CREATE TABLE auth_flows (
  id_hash    TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('oauth', 'signup', 'app_code')),
  data       TEXT NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX auth_flows_expires ON auth_flows(expires_at);

CREATE TABLE clips (
  id          TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  title       TEXT NOT NULL,
  game        TEXT,
  visibility  TEXT NOT NULL CHECK (visibility IN ('private', 'unlisted', 'public')),
  status      TEXT NOT NULL CHECK (status IN ('uploading', 'ready')),
  -- Tamanho do vídeo declarado na criação (reserva a cota) e conferido no fim do upload.
  size_bytes  INTEGER NOT NULL,
  thumb_bytes INTEGER NOT NULL DEFAULT 0,
  duration_ms INTEGER NOT NULL,
  width       INTEGER NOT NULL,
  height      INTEGER NOT NULL,
  -- Multipart do R2 enquanto status = 'uploading'.
  upload_id   TEXT,
  created_at  INTEGER NOT NULL,
  ready_at    INTEGER
);
CREATE INDEX clips_user ON clips(user_id, created_at DESC);
CREATE INDEX clips_status ON clips(status, created_at);

CREATE TABLE clip_parts (
  clip_id     TEXT NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
  part_number INTEGER NOT NULL,
  etag        TEXT NOT NULL,
  PRIMARY KEY (clip_id, part_number)
);

-- Janela fixa por chave (ex.: "convite:<ip>"). Limpo pela faxina diária.
CREATE TABLE rate_limits (
  key          TEXT NOT NULL,
  window_start INTEGER NOT NULL,
  count        INTEGER NOT NULL,
  PRIMARY KEY (key, window_start)
);
