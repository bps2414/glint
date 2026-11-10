-- Favorito no servidor (espelha o favorito do app). Quando o espaço acaba, o app sugere apagar
-- os clipes mais antigos do usuário, e um favorito nunca entra na sugestão.
ALTER TABLE clips ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
