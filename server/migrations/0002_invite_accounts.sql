-- Convite vira conta: `users.discord_id` passa a ser opcional e ganha `name` (o nome mostrado no
-- servidor). Contas por convite usam `username = name` (o app antigo exige `username`).
-- `auth_flows` ganha os tipos 'link' (código para outro PC) e 'web_code' (app abre o site logado).
--
-- SQLite não tira NOT NULL sem reconstruir a tabela, e no D1 o DROP TABLE de `users` dispara o
-- ON DELETE CASCADE de `sessions` e `clips` (defer_foreign_keys não impede). Por isso os dados vão
-- para cópias sem chave estrangeira, as tabelas são derrubadas dos filhos para o pai e recriadas.
PRAGMA defer_foreign_keys = on;

CREATE TABLE bak_users AS SELECT * FROM users;
CREATE TABLE bak_invites AS SELECT * FROM invites;
CREATE TABLE bak_sessions AS SELECT * FROM sessions;
CREATE TABLE bak_clips AS SELECT * FROM clips;
CREATE TABLE bak_clip_parts AS SELECT * FROM clip_parts;
CREATE TABLE bak_auth_flows AS SELECT * FROM auth_flows;

DROP TABLE clip_parts;
DROP TABLE clips;
DROP TABLE sessions;
DROP TABLE invites;
DROP TABLE users;
DROP TABLE auth_flows;

CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  -- NULL = conta criada por convite (sem Discord).
  discord_id    TEXT UNIQUE,
  name          TEXT NOT NULL,
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

-- Etapas curtas do login. Cada linha é usada uma vez só (DELETE ... RETURNING).
CREATE TABLE auth_flows (
  id_hash    TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('oauth', 'signup', 'app_code', 'link', 'web_code')),
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

INSERT INTO users (id, discord_id, name, username, display_name, avatar, role, quota_bytes, disabled, invite_id, created_at, last_login_at)
  SELECT id, discord_id, COALESCE(display_name, username), username, display_name, avatar, role, quota_bytes, disabled, invite_id, created_at, last_login_at
    FROM bak_users;
INSERT INTO invites SELECT id, code_hash, hint, note, created_by, created_at, expires_at, max_uses, uses, revoked_at FROM bak_invites;
INSERT INTO sessions SELECT token_hash, user_id, kind, created_at, expires_at, last_used_at FROM bak_sessions;
INSERT INTO auth_flows SELECT id_hash, kind, data, expires_at FROM bak_auth_flows;
INSERT INTO clips SELECT id, user_id, title, game, visibility, status, size_bytes, thumb_bytes, duration_ms, width, height, upload_id, created_at, ready_at FROM bak_clips;
INSERT INTO clip_parts SELECT clip_id, part_number, etag FROM bak_clip_parts;

DROP TABLE bak_clip_parts;
DROP TABLE bak_clips;
DROP TABLE bak_auth_flows;
DROP TABLE bak_sessions;
DROP TABLE bak_invites;
DROP TABLE bak_users;
