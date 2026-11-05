#!/usr/bin/env bash
# Renomeia o app (marca) nos poucos lugares onde ela aparece.
# Uso: scripts/rename-brand.sh NovoNome
# Os crates internos (app-core, app-media, ...) não têm marca e não mudam.
set -euo pipefail
new_name="${1:?uso: $0 NovoNome}"
new_slug="$(echo "$new_name" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9-')"
root="$(cd "$(dirname "$0")/.." && pwd)"
old_name="$(sed -n 's/^pub const APP_NAME: &str = "\(.*\)";/\1/p' "$root/crates/app-core/src/brand.rs")"
old_slug="$(sed -n 's/^pub const APP_SLUG: &str = "\(.*\)";/\1/p' "$root/crates/app-core/src/brand.rs")"
echo "Renomeando '$old_name' ($old_slug) -> '$new_name' ($new_slug)"
sed -i "s/^pub const APP_NAME: &str = \".*\";/pub const APP_NAME: \&str = \"$new_name\";/; s/^pub const APP_SLUG: &str = \".*\";/pub const APP_SLUG: \&str = \"$new_slug\";/" "$root/crates/app-core/src/brand.rs"
sed -i "s/^name = \"$old_slug-recorder\"/name = \"$new_slug-recorder\"/" "$root/crates/app-recorder/Cargo.toml"
sed -i "s/^name = \"$old_slug\"/name = \"$new_slug\"/" "$root/crates/app-ui/Cargo.toml"
sed -i "s/^  APP_NAME: .*/  APP_NAME: $new_name/; s/^  APP_SLUG: .*/  APP_SLUG: $new_slug/" "$root/.github/workflows/ci.yml"
# Servidor: só o nome exibido. O nome do Worker ("clips", em server/wrangler.jsonc) é
# separado da marca: a URL é clips.<subdomínio da conta>.workers.dev e não muda ao renomear.
sed -i "s/^export const APP_NAME = \".*\";/export const APP_NAME = \"$new_name\";/" "$root/server/src/brand.ts"
echo "Pronto. Revise com: git diff"
echo "Obs.: a pasta de config/clipes muda para %APPDATA%\\$new_name e Vídeos\\$new_name."
