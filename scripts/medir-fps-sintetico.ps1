# Mede o custo do gravador no FPS sem precisar de jogo nem de pessoa: abre o "jogo" sintético
# (exemplo carga-gpu, sem limite de FPS), mede por ETW sem → com (janela) → sem → com (monitor) → sem
# e compara cada "com" com a média das medidas "sem" vizinhas. Precisa de administrador (ETW).
# Uso (pwsh, na raiz do repo, depois do build-windows.ps1):
#   cargo build --release -p app-platform-win --example carga-gpu
#   scripts\medir-fps-sintetico.ps1 [-Segundos 30] [-Carga 220]
param([int]$Segundos = 30, [int]$Carga = 220)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)

$rec = (Resolve-Path "dist\Glint-fase5\glint-recorder.exe").Path
$cargaExe = (Resolve-Path "target\release\examples\carga-gpu.exe").Path
$cfg = "$env:APPDATA\Glint\config\config.toml"
$cfgOrig = Get-Content $cfg -Raw

function Estado { (& $rec --status 2>&1 | Out-String) }
function Fechar { & $rec --quit 2>&1 | Out-Null; while (Get-Process glint-recorder -ErrorAction SilentlyContinue) { Start-Sleep 1 } }
function Medir([int]$jogo) {
  $kv = @{}
  & $rec --measure-fps $jogo --secs $Segundos 2>$null | ForEach-Object { if ($_ -match "^(\w+)=(.+)$") { $kv[$Matches[1]] = $Matches[2] } }
  [pscustomobject]@{ Fps = [double]$kv["fps_avg"]; Low1 = [double]$kv["fps_low1"]; Pior = [double]$kv["worst_ms"] }
}
function Gravando([string]$modo, [int]$jogo) {
  Set-Content $cfg ($cfgOrig -replace 'capture = "\w+"', "capture = `"$modo`"") -NoNewline
  Start-Process $rec
  for ($i = 0; $i -lt 30 -and (Estado) -notmatch "Estado:\s+gravando"; $i++) { Start-Sleep 1 }
  $st = Estado
  if ($st -notmatch "Estado:\s+gravando") { throw "o gravador não começou a gravar a carga ($modo)" }
  Start-Sleep 3   # deixa o encoder estabilizar
  $p = Get-Process glint-recorder | Select-Object -First 1
  $t0 = $p.TotalProcessorTime; $w0 = Get-Date
  $m = Medir $jogo
  $p.Refresh()
  $cpu = 100 * ($p.TotalProcessorTime - $t0).TotalSeconds / ((Get-Date) - $w0).TotalSeconds / [Environment]::ProcessorCount
  $video = ([regex]::Match($st, "(?m)^Vídeo:\s*(.+)$")).Groups[1].Value.Trim()
  Fechar
  $m | Add-Member Cpu $cpu -PassThru | Add-Member Video $video -PassThru
}

Fechar
$j = Start-Process $cargaExe -ArgumentList $Carga, 0 -PassThru
Start-Sleep 3
try {
  $res = [ordered]@{}
  $res["sem1"] = Medir $j.Id
  $res["janela"] = Gravando "window" $j.Id
  $res["sem2"] = Medir $j.Id
  $res["monitor"] = Gravando "monitor" $j.Id
  $res["sem3"] = Medir $j.Id
} finally {
  Stop-Process $j -ErrorAction SilentlyContinue
  Fechar
  Set-Content $cfg $cfgOrig -NoNewline
}

foreach ($k in $res.Keys) {
  $m = $res[$k]
  $extra = if ($m.Video) { " | gravador CPU {0:N1}% | {1}" -f $m.Cpu, $m.Video } else { "" }
  "{0,-8} FPS {1,6:N1} | 1% low {2,6:N1} | pior {3,6:N1} ms{4}" -f $k, $m.Fps, $m.Low1, $m.Pior, $extra
}
foreach ($par in @(@("janela", "sem1", "sem2"), @("monitor", "sem2", "sem3"))) {
  $base = ($res[$par[1]].Fps + $res[$par[2]].Fps) / 2
  $baseLow = ($res[$par[1]].Low1 + $res[$par[2]].Low1) / 2
  "Perda {0,-8} FPS médio {1:N1}% | 1% low {2:N1}%" -f $par[0], (100 * (1 - $res[$par[0]].Fps / $base)), (100 * (1 - $res[$par[0]].Low1 / $baseLow))
}
exit 0   # o último --quit (gravador já fechado) deixaria código 1
