# Build, testes e pacote do Windows NO PC (substitui o job Windows do CI).
# Uso (PowerShell, na raiz do repo):  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
# Opções: -SkipTests  (só build + pacote)
# Pré-requisitos: Rust (rustup, toolchain MSVC), Visual Studio Build Tools com C++, LLVM (libclang para o bindgen).
param([switch]$SkipTests)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$AppName = "Glint"; $AppSlug = "glint"
$FfmpegMax = [version]"8.1"   # maior série aceita pelo ffmpeg-sys-next 9 (igual ao CI)

# --- FFmpeg LGPL shared (BtbN), baixado uma vez para .deps\ ---
$deps = Join-Path (Get-Location) ".deps"
$ff = Get-ChildItem $deps -Directory -Filter "ffmpeg-n*-lgpl-shared*" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $ff) {
  New-Item -ItemType Directory -Force $deps | Out-Null
  Write-Host "Baixando FFmpeg LGPL (BtbN)..."
  $rel = Invoke-RestMethod "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest" -Headers @{ "User-Agent" = "build-windows" }
  $best = $null; $bestVer = [version]"0.0"
  foreach ($a in $rel.assets) {
    if ($a.name -match '^ffmpeg-n(\d+)\.(\d+)-latest-win64-lgpl-shared-\d+\.\d+\.zip$') {
      $v = [version]"$($Matches[1]).$($Matches[2])"
      if ($v -le $FfmpegMax -and $v -gt $bestVer) { $best = $a; $bestVer = $v }
    }
  }
  if (-not $best) { throw "nenhum FFmpeg LGPL shared <= $FfmpegMax no release do BtbN" }
  $zip = Join-Path $deps $best.name
  Invoke-WebRequest $best.browser_download_url -OutFile $zip
  Expand-Archive $zip -DestinationPath $deps -Force
  Remove-Item $zip
  $ff = Get-ChildItem $deps -Directory -Filter "ffmpeg-n*-lgpl-shared*" | Select-Object -First 1
}
Write-Host "FFmpeg: $($ff.FullName)"
$env:FFMPEG_DIR = $ff.FullName
$env:PATH = "$($ff.FullName)\bin;$env:PATH"
if (-not $env:LIBCLANG_PATH) {
  foreach ($p in @("C:\Program Files\LLVM\bin", "C:\Program Files (x86)\LLVM\bin")) {
    if (Test-Path "$p\libclang.dll") { $env:LIBCLANG_PATH = $p; break }
  }
}
if (-not $env:LIBCLANG_PATH) { throw "libclang.dll não encontrado: instale o LLVM (winget install LLVM.LLVM)" }

# --- Build e testes (mesmo conjunto de pacotes: as dependências não recompilam entre os dois) ---
$pkgs = @("-p", "app-core", "-p", "app-media", "-p", "app-ipc", "-p", "app-platform-win", "-p", "app-recorder", "-p", "app-ui")
cargo build --release -p app-recorder -p app-ui
if ($LASTEXITCODE -ne 0) { throw "build falhou" }
if (-not $SkipTests) {
  cargo test --release @pkgs
  if ($LASTEXITCODE -ne 0) { throw "testes falharam" }
}

# --- Pacote (igual ao do CI) ---
$out = "dist\$AppName-fase5"
# Esvazia em vez de apagar a pasta: um Explorer aberto nela não trava o build.
if (Test-Path $out) { Get-ChildItem $out -Force | Remove-Item -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "target\release\$AppSlug-recorder.exe" $out
Copy-Item "target\release\$AppSlug.exe" $out
Copy-Item "$($ff.FullName)\bin\*.dll" $out
Copy-Item "$($ff.FullName)\LICENSE.txt" "$out\FFMPEG-LICENSE.txt" -ErrorAction SilentlyContinue
Copy-Item crates\app-ui\assets\fonts\OFL.txt "$out\FONTES-OFL.txt"
Copy-Item README.pt-BR.md "$out\LEIA-ME.md"
Copy-Item scripts\TESTE.bat, scripts\teste-guiado.ps1 $out
New-Item -ItemType Directory -Force "$out\avancado" | Out-Null
Copy-Item scripts\avancado\*.bat "$out\avancado"
Write-Host "`nPacote pronto em $out"
