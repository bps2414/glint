# Instalador do Glint no PC (só para build/teste local): build + testes e instalador (Inno Setup).
# O release de verdade sai pelo CI: o push de uma tag `vX.Y.Z` dispara .github/workflows/release.yml
# (instalador + latest.json no GitHub Releases, de onde o app se atualiza).
# Uso: powershell -ExecutionPolicy Bypass -File scripts\release.ps1 [-SkipTests]
param([switch]$SkipTests)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Glint $version"

# 1. Build + pacote (dist\Glint-fase5) pelo script de sempre.
$args = @(); if ($SkipTests) { $args += "-SkipTests" }
& powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 @args
if ($LASTEXITCODE -ne 0) { throw "build falhou" }

# 2. Pasta só com o que o usuário precisa (sem os scripts de teste).
$src = "dist\Glint"
if (Test-Path $src) { Remove-Item $src -Recurse -Force }
New-Item -ItemType Directory $src | Out-Null
Copy-Item dist\Glint-fase5\*.exe, dist\Glint-fase5\*.dll, dist\Glint-fase5\*.txt $src

# 3. Instalador.
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "C:\Program Files (x86)\Inno Setup 6\ISCC.exe", "C:\Program Files\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
  Write-Host "Instalando o Inno Setup (winget)..."
  winget install --id JRSoftware.InnoSetup -e --silent --accept-package-agreements --accept-source-agreements
  $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "C:\Program Files (x86)\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
  if (-not $iscc) { throw "ISCC.exe não encontrado depois do winget" }
}
& $iscc "/DAppVersion=$version" "/DSrcDir=..\dist\Glint" installer\glint.iss
if ($LASTEXITCODE -ne 0) { throw "Inno Setup falhou" }
$setup = "dist\Glint-Setup-$version.exe"
Write-Host "`nInstalador: $setup"
