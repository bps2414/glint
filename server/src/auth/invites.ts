// Convites: o código aparece uma vez (para o admin copiar); o banco guarda só o hash.
import { isToken, randomId, randomToken, sha256 } from "../lib/crypto";

export async function createInvite(
  db: D1Database,
  createdBy: string,
  opts: { note: string | null; maxUses: number; expiresAt: number | null },
  now: number,
): Promise<{ id: string; code: string }> {
  const code = randomToken(16);
  const id = randomId();
  await db
    .prepare(
      `INSERT INTO invites (id, code_hash, hint, note, created_by, created_at, expires_at, max_uses)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
    )
    .bind(id, await sha256(code), code.slice(0, 4), opts.note, createdBy, now, opts.expiresAt, opts.maxUses)
    .run();
  return { id, code };
}

/** Aceita o código puro ou o link inteiro (`.../i/<código>`). */
export function parseInviteCode(input: string | undefined | null): string | null {
  if (!input) return null;
  const s = input.trim();
  const m = /\/i\/([A-Za-z0-9_-]+)\/?$/.exec(s);
  const code = m ? m[1] : s;
  return isToken(code, 22, 22) ? code : null;
}

/** Convite ainda valendo (sem gastar): devolve o nome de quem convidou, ou `null`. */
export async function peekInvite(db: D1Database, code: string, now: number): Promise<{ inviter: string } | null> {
  const row = await db
    .prepare(
      `SELECT u.name AS inviter FROM invites i JOIN users u ON u.id = i.created_by
        WHERE i.code_hash = ? AND i.revoked_at IS NULL AND i.uses < i.max_uses
          AND (i.expires_at IS NULL OR i.expires_at > ?)`,
    )
    .bind(await sha256(code), now)
    .first<{ inviter: string }>();
  return row ?? null;
}

/** Gasta um uso do convite, de forma atômica. Devolve o id do convite ou `null`. */
export async function redeemInvite(db: D1Database, code: string, now: number): Promise<string | null> {
  const row = await db
    .prepare(
      `UPDATE invites SET uses = uses + 1
        WHERE code_hash = ? AND revoked_at IS NULL AND uses < max_uses
          AND (expires_at IS NULL OR expires_at > ?)
        RETURNING id`,
    )
    .bind(await sha256(code), now)
    .first<{ id: string }>();
  return row?.id ?? null;
}

/** Devolve o uso (se o cadastro falhar depois de gastar o convite). */
export async function refundInvite(db: D1Database, id: string) {
  await db.prepare("UPDATE invites SET uses = MAX(uses - 1, 0) WHERE id = ?").bind(id).run();
}
