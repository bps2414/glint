/** @jsxImportSource hono/jsx */
import type { User } from "../env";
import { t } from "../i18n";
import { formatBytes, formatDate } from "../lib/format";
import { Layout } from "./layout";

export interface InviteRow {
  id: string;
  hint: string;
  note: string | null;
  created_at: number;
  expires_at: number | null;
  max_uses: number;
  uses: number;
  revoked_at: number | null;
}

export interface UserRow extends User {
  used: number;
  clips: number;
}

function inviteState(i: InviteRow, now: number): string {
  if (i.revoked_at) return t.inviteRevoked;
  if (i.expires_at && i.expires_at <= now) return t.inviteExpired;
  if (i.uses >= i.max_uses) return t.inviteUsedUp;
  return t.inviteActive;
}

export function AdminPage(props: {
  me: User;
  invites: InviteRow[];
  users: UserRow[];
  now: number;
  /** Link do convite recém-criado (mostrado uma vez). */
  created?: string;
  /** Cota por usuário ligada (USER_QUOTA_GB); `quota_bytes` dos usuários já vem efetivo. */
  quotaOn: boolean;
  /** Espaço usado no servidor todo e a trava global. */
  total: number;
  limit: number;
}) {
  return (
    <Layout title={t.admin} user={props.me} scripts={["clip.js"]} wide>
      <section class="card">
        <h2>{t.newInvite}</h2>
        {props.created && (
          <div class="created">
            <p>{t.inviteCreated}</p>
            <div class="copy-row">
              <input readonly value={props.created} class="mono" />
              <button class="btn" type="button" data-copy={props.created} data-copied={t.copied}>
                {t.copyLink}
              </button>
            </div>
          </div>
        )}
        <form method="post" action="/admin/convites" class="row-form">
          <label>
            {t.inviteNote}
            <input name="note" maxlength={60} />
          </label>
          <label class="small">
            {t.inviteMaxUses}
            <input name="max_uses" type="number" min={1} max={100} value="1" required />
          </label>
          <label class="small">
            {t.inviteDays}
            <input name="days" type="number" min={0} max={365} value="7" required />
          </label>
          <button class="btn" type="submit">
            {t.inviteCreate}
          </button>
        </form>
      </section>

      <section class="card">
        <h2>{t.invites}</h2>
        <table class="table">
          <tbody>
            {props.invites.map((i) => {
              const state = inviteState(i, props.now);
              return (
                <tr>
                  <td class="mono">{i.hint}…</td>
                  <td>{i.note ?? ""}</td>
                  <td class="mono">
                    {i.uses}/{i.max_uses}
                  </td>
                  <td class="muted">{i.expires_at ? formatDate(i.expires_at) : "—"}</td>
                  <td>
                    <span class={state === t.inviteActive ? "tag on" : "tag"}>{state}</span>
                  </td>
                  <td class="right">
                    {state === t.inviteActive && (
                      <form method="post" action={`/admin/convites/${i.id}/revogar`} class="inline">
                        <button class="link danger" type="submit">
                          {t.inviteRevoke}
                        </button>
                      </form>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>

      <section class="card">
        <h2>{t.users}</h2>
        <p class="muted">
          {t.serverStorage}: <span class="mono">{t.usage(formatBytes(props.total), formatBytes(props.limit))}</span>
        </p>
        <table class="table">
          <tbody>
            {props.users.map((u) => (
              <tr class={u.disabled ? "off" : ""}>
                <td>
                  {u.display_name ?? u.username}
                  {u.role === "admin" && <span class="tag on">admin</span>}
                  {u.id === props.me.id && <span class="muted"> ({t.you})</span>}
                </td>
                <td class="mono">
                  {props.quotaOn ? `${formatBytes(u.used)} / ${formatBytes(u.quota_bytes)}` : formatBytes(u.used)}
                </td>
                <td class="mono">{u.clips}</td>
                <td>
                  {props.quotaOn && (
                    <form method="post" action={`/admin/usuarios/${u.id}/cota`} class="inline">
                      <input
                        name="quota_gb"
                        type="number"
                        min={0}
                        max={10000}
                        step="any"
                        value={String(Math.round((u.quota_bytes / 1024 ** 3) * 100) / 100)}
                        aria-label={t.quotaGb}
                        class="tiny"
                      />
                      <button class="link" type="submit">
                        {t.save}
                      </button>
                    </form>
                  )}
                </td>
                <td class="right">
                  {u.id !== props.me.id && (
                    <form
                      method="post"
                      action={`/admin/usuarios/${u.id}/${u.disabled ? "reativar" : "desativar"}`}
                      class="inline"
                    >
                      <button class={u.disabled ? "link" : "link danger"} type="submit">
                        {u.disabled ? t.enable : t.disable}
                      </button>
                    </form>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </Layout>
  );
}
