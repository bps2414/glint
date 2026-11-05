// OAuth2 do Discord (escopo "identify": só id, nome e avatar; sem e-mail).

export interface DiscordProfile {
  id: string;
  username: string;
  global_name: string | null;
  avatar: string | null;
}

export interface DiscordApi {
  authorizeUrl(p: { clientId: string; redirectUri: string; state: string }): string;
  /** Troca o `code` pelo perfil do usuário (o token do Discord não é guardado). */
  profile(p: {
    clientId: string;
    clientSecret: string;
    redirectUri: string;
    code: string;
  }): Promise<DiscordProfile>;
}

const API = "https://discord.com/api/v10";

export class DiscordError extends Error {}

export const realDiscord: DiscordApi = {
  authorizeUrl({ clientId, redirectUri, state }) {
    const u = new URL("https://discord.com/oauth2/authorize");
    u.searchParams.set("response_type", "code");
    u.searchParams.set("client_id", clientId);
    u.searchParams.set("scope", "identify");
    u.searchParams.set("redirect_uri", redirectUri);
    u.searchParams.set("state", state);
    // Não pede de novo a autorização para quem já autorizou.
    u.searchParams.set("prompt", "none");
    return u.toString();
  },

  async profile({ clientId, clientSecret, redirectUri, code }) {
    const tokenRes = await fetch(`${API}/oauth2/token`, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "authorization_code",
        code,
        redirect_uri: redirectUri,
        client_id: clientId,
        client_secret: clientSecret,
      }),
    });
    if (!tokenRes.ok) throw new DiscordError(`token: HTTP ${tokenRes.status}`);
    const token = (await tokenRes.json()) as { access_token?: string; token_type?: string };
    if (!token.access_token) throw new DiscordError("token: resposta sem access_token");

    const meRes = await fetch(`${API}/users/@me`, {
      headers: { Authorization: `Bearer ${token.access_token}` },
    });
    if (!meRes.ok) throw new DiscordError(`users/@me: HTTP ${meRes.status}`);
    const me = (await meRes.json()) as Partial<DiscordProfile>;
    if (typeof me.id !== "string" || !/^\d{5,25}$/.test(me.id) || typeof me.username !== "string") {
      throw new DiscordError("users/@me: perfil inválido");
    }
    return {
      id: me.id,
      username: me.username.slice(0, 64),
      global_name: typeof me.global_name === "string" ? me.global_name.slice(0, 64) : null,
      avatar: typeof me.avatar === "string" && /^(a_)?[0-9a-f]{16,64}$/.test(me.avatar) ? me.avatar : null,
    };
  },
};

export function avatarUrl(discordId: string, avatar: string | null): string | null {
  if (!avatar) return null;
  return `https://cdn.discordapp.com/avatars/${discordId}/${avatar}.png?size=64`;
}
