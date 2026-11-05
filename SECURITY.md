# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue.

Use GitHub's private reporting: on the repository, go to **Security -> Report a vulnerability**
(<https://github.com/bps2414/glint/security/advisories/new>). This opens a private advisory only the
maintainer can see.

Include what you found, how to reproduce it, the affected version (or commit) and the impact you expect. I
will acknowledge the report as soon as I can, keep you posted while I fix it, and credit you in the advisory
if you want.

## Scope

- The Windows app (`glint`, `glint-recorder`) and the installer.
- The self-hosted server in `server/` (login, invites, upload, private clip URLs, admin pages).

Each person who deploys the server runs their own copy, so also say whether the problem needs a particular
configuration.

## Supported versions

Only the latest release gets security fixes.
