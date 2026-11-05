# Run your own Glint server

The app works fully offline. A server is only needed to publish clips and get a share link. You can run your
own on Cloudflare's free tier (Workers + D1 + R2); each group of friends uses its own account.

Sign-in is Discord only, so you need a Discord app first.

## 1. Create the Discord app

1. Open the [Discord Developer Portal](https://discord.com/developers/applications) and create an application.
2. Copy the **Application ID** (General Information) and the **Client Secret** (OAuth2).
3. After the server is deployed (you only know its URL then), go to OAuth2, Redirects and add:
   `https://<your-worker>.workers.dev/auth/discord/callback`

## 2. Deploy

### Option A: the button

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/bps2414/glint/tree/main/server)

Cloudflare copies the `server/` folder to your GitHub account, creates the D1 database and the R2 bucket from
`wrangler.jsonc`, and asks for the secrets and variables below. The deploy command (`npm run deploy`) applies the
database migrations and publishes the Worker. If the setup page shows a different command, use
`npx wrangler d1 migrations apply DB --remote && npx wrangler deploy`.

### Option B: from your computer

Needs Node 22+ and a Cloudflare account.

```sh
git clone https://github.com/bps2414/glint && cd glint/server
npm ci
npx wrangler login
DISCORD_CLIENT_ID=... DISCORD_CLIENT_SECRET=... npm run deploy
```

`scripts/deploy.mjs` creates the D1 database and R2 bucket if they are missing, applies the migrations, generates
`SIGNING_KEY` on the first run, uploads the secrets and publishes. It prints the URL to use in the Discord redirect.
Run it again to update; existing secrets are kept. Variables come from `wrangler.jsonc`; the ones you pass in the
environment (`DISCORD_CLIENT_ID`, `ADMIN_DISCORD_IDS`, `INVITE_REQUIRED`, `USER_QUOTA_GB`, `STORAGE_LIMIT_GB`, `SERVER_LANG`) override them.

## 3. Variables

Variables live in `server/wrangler.jsonc` (`vars`); secrets are set by the button or by `npm run deploy`. Change a
variable there (in your copy of the repository, for the button) and deploy again.

| Name | Default | Meaning |
|---|---|---|
| `DISCORD_CLIENT_ID` | none (required) | Application ID of your Discord app |
| `DISCORD_CLIENT_SECRET` | none (secret, required) | Client Secret of your Discord app |
| `SIGNING_KEY` | none (secret, required) | 32+ random characters; signs links to private clips |
| `STORAGE_LIMIT_GB` | `9.5` | Global limit: new clips are refused (HTTP 507, `storage_full`) when all clips together would pass it. R2 is free up to 10 GB |
| `USER_QUOTA_GB` | `0` | Per-user quota in GB. `0` or unset means no per-user quota |
| `INVITE_REQUIRED` | `true` | `false` lets any Discord account sign up |
| `ADMIN_DISCORD_IDS` | empty | Comma-separated Discord user IDs that are always admin |
| `MAX_CLIP_MB` | `500` | Largest single clip |
| `SERVER_LANG` | `en` | Language of the web pages and API messages: `en` or `pt-BR` |

## 4. First login and invites

On a new server (no users yet, `ADMIN_DISCORD_IDS` empty) the first Discord login becomes the admin, with no
invite. Sign in yourself right after deploying, before sharing the URL. From then on everyone needs an invite.

To invite someone: open `https://<your-worker>.workers.dev/admin`, fill in "New invite" (note, number of uses,
days) and send them the link it shows. The link is shown once. In the same page you can revoke invites, disable
users and, if `USER_QUOTA_GB` is on, change each user's quota.

## 5. Connect the app

In the app, open Settings, Server, paste your server URL (for example `https://clips.example.workers.dev`) and sign
in with Discord. Publishing then works; without a URL the app stays 100% local.

## Updating the server

Pull the new version and deploy again (the button creates a repository you can sync, Option B is `git pull` and
`npm run deploy`). Migrations in `server/migrations/` are applied automatically on deploy.
