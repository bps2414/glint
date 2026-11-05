//! Ícone e informações de versão do .exe (Windows). O nome do app vem de
//! `app-core/src/brand.rs` (lido como texto, como o `scripts/rename-brand.sh`), para a
//! marca continuar num lugar só. Fora do Windows não faz nada.

use std::path::PathBuf;

/// Descrição do arquivo (aparece em Propriedades → Detalhes e no Gerenciador de Tarefas).
const DESCRIPTION: &str = "gravador em segundo plano";

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo"));
    let root = manifest.join("../..");
    let brand = root.join("crates/app-core/src/brand.rs");
    let icon = root.join("assets/brand/glint.ico");
    println!("cargo:rerun-if-changed={}", brand.display());
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let text = std::fs::read_to_string(&brand).expect("brand.rs");
    let name = text
        .lines()
        .find_map(|l| l.strip_prefix("pub const APP_NAME: &str = \""))
        .and_then(|l| l.strip_suffix("\";"))
        .expect("APP_NAME em brand.rs");
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo");
    let nums: Vec<u32> = version
        .split(['.', '-'])
        .take(3)
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    let v = format!("{},{},{},0", nums[0], nums[1], nums[2]);
    let icon = icon
        .canonicalize()
        .expect("assets/brand/glint.ico (rode cargo run -p make-icons)");
    let icon = icon.display().to_string().replace('\\', "/");
    // O GPUI carrega o ícone da janela pelo recurso 1.
    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {v}
PRODUCTVERSION {v}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041604B0"
    BEGIN
      VALUE "CompanyName", "{name}"
      VALUE "FileDescription", "{name} - {DESCRIPTION}"
      VALUE "FileVersion", "{version}"
      VALUE "ProductName", "{name}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0416, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo")).join("app.rc");
    std::fs::write(&out, rc).expect("app.rc");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("recursos do Windows (ícone/versão)");
}
