# Teste guiado do app: roda tudo em etapas, mede sozinho e junta num relatório só.
# Aberto pelo TESTE.bat (que pede administrador: medir FPS por ETW exige).
# Compatível com Windows PowerShell 5.1.
param([string]$Jogo = "VALORANT-Win64-Shipping", [int]$Segundos = 60)

$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$OutputEncoding = [Text.Encoding]::UTF8
$PSDefaultParameterValues["*-Content:Encoding"] = "UTF8"   # 5.1 gravaria o relatório em ANSI
Set-Location $PSScriptRoot

$Rec = (Get-ChildItem -Filter "*-recorder.exe" | Select-Object -First 1).FullName
$App = $Rec -replace "-recorder\.exe$", ".exe"
$Nome = [IO.Path]::GetFileNameWithoutExtension($App)
$Site = "https://clips.getglint.workers.dev"
$Stamp = Get-Date -Format "yyyyMMdd-HHmm"
$Desk = [Environment]::GetFolderPath("Desktop")
$Rel = Join-Path $Desk "$Nome-teste-$Stamp.txt"
$Tmp = Join-Path $env:TEMP "$Nome-teste"
New-Item -ItemType Directory -Force $Tmp | Out-Null

Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Fg {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
}
"@

# ---------- utilidades ----------
$voz = $null
try {
  Add-Type -AssemblyName System.Speech
  $voz = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $pt = $voz.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like "pt*" } | Select-Object -First 1
  if ($pt) { $voz.SelectVoice($pt.VoiceInfo.Name) } else { $voz = $null }
} catch { $voz = $null }

# Fala (se houver voz em português) e bipa: avisa mesmo com o jogo na frente.
function Avisar([string]$texto) {
  [Console]::Beep(880, 150); [Console]::Beep(1320, 150)
  Write-Host "  >> $texto" -ForegroundColor Yellow
  if ($voz) { try { $voz.Speak($texto) } catch {} }
}
function Titulo([string]$t) {
  Write-Host ""; Write-Host ("=" * 64) -ForegroundColor DarkGray
  Write-Host " $t" -ForegroundColor Cyan
  Write-Host ("=" * 64) -ForegroundColor DarkGray
  Add-Content $Rel "`r`n===== $t ====="
}
function Anotar([string]$t) { Add-Content $Rel $t; Write-Host "  $t" }
function Perguntar([string]$p) {
  $r = Read-Host "  $p"
  Add-Content $Rel "P: $p`r`nR: $r"
  return $r
}
function SimNao([string]$p) {
  do { $r = (Read-Host "  $p [s/n]").Trim().ToLower() } until ($r -in @("s", "n", "sim", "nao", "não"))
  $ok = $r.StartsWith("s")
  Add-Content $Rel ("P: $p`r`nR: " + $(if ($ok) { "sim" } else { "não" }))
  return $ok
}
function Rec([string[]]$a) { (& $Rec @a 2>&1 | Out-String).Trim() }
function Estado { Rec @("--status") }
function Campo([string]$status, [string]$nome) {
  $m = [regex]::Match($status, "(?m)^$nome\:\s*(.+)$")
  if ($m.Success) { $m.Groups[1].Value.Trim() } else { "" }
}
function Subir-Gravador {
  if ((Estado) -notmatch "Estado:") {
    Start-Process $Rec | Out-Null
    for ($i = 0; $i -lt 20; $i++) { Start-Sleep 1; if ((Estado) -match "Estado:") { return $true } }
    return $false
  }
  return $true
}
function Fechar-Gravador {
  Rec @("--quit") | Out-Null
  for ($i = 0; $i -lt 10 -and (Get-Process ([IO.Path]::GetFileNameWithoutExtension($Rec)) -ErrorAction SilentlyContinue); $i++) { Start-Sleep 1 }
}
function Pid-Em-Foco {
  $p = [uint32]0; [void][Fg]::GetWindowThreadProcessId([Fg]::GetForegroundWindow(), [ref]$p); return [int]$p
}
function Tela-Cheia {
  $h = [Fg]::GetForegroundWindow(); $r = New-Object Fg+RECT
  [void][Fg]::GetWindowRect($h, [ref]$r)
  $s = [System.Windows.Forms.Screen]::FromHandle($h).Bounds
  return ($r.L -le $s.Left -and $r.T -le $s.Top -and $r.R -ge $s.Right -and $r.B -ge $s.Bottom)
}

# Mede o FPS do processo (ETW) e, junto, CPU/memória/GPU do gravador (se $recPid).
function Medir([int]$jogoPid, [int]$secs, [int]$recPid = 0) {
  $out = Join-Path $Tmp "fps.txt"
  for ($tent = 0; $tent -lt 2; $tent++) {
    $p = Start-Process $Rec -ArgumentList "--measure-fps", $jogoPid, "--secs", $secs -NoNewWindow -PassThru -RedirectStandardOutput $out
    $cpu = @(); $mem = @(); $enc = @(); $g3d = @(); $foco = 0; $amostras = 0
    $cores = [Environment]::ProcessorCount
    $prev = $null; $prevT = Get-Date
    while (-not $p.HasExited) {
      Start-Sleep 2
      $amostras++; if ((Pid-Em-Foco) -eq $jogoPid) { $foco++ }
      if ($recPid) {
        $rp = Get-Process -Id $recPid -ErrorAction SilentlyContinue
        if ($rp) {
          $now = Get-Date
          if ($prev) { $cpu += 100 * ($rp.TotalProcessorTime - $prev).TotalSeconds / ($now - $prevT).TotalSeconds / $cores }
          $prev = $rp.TotalProcessorTime; $prevT = $now; $mem += $rp.WorkingSet64 / 1MB
          # Classe CIM em vez de Get-Counter: os nomes dos contadores mudam com o idioma do Windows.
          try {
            $c = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction Stop |
              Where-Object { $_.Name -like "pid_$($recPid)_*" }
            $enc += ($c | Where-Object { $_.Name -match "encode|codec" } | Measure-Object UtilizationPercentage -Sum).Sum
            $g3d += ($c | Where-Object { $_.Name -match "engtype_3D$" } | Measure-Object UtilizationPercentage -Sum).Sum
          } catch {}
        }
      }
    }
    $kv = @{}
    Get-Content $out -ErrorAction SilentlyContinue | ForEach-Object { if ($_ -match "^(\w+)=(.+)$") { $kv[$Matches[1]] = $Matches[2] } }
    $focoPct = if ($amostras) { 100 * $foco / $amostras } else { 0 }
    # Medida suja (carregamento, menu, alt-tab): quadro de mais de 1 s ou jogo fora de foco. Refaz uma vez.
    $temQuadros = $kv["frames"] -and [int]$kv["frames"] -gt 0
    $limpa = $temQuadros -and [double]$kv["worst_ms"] -lt 1000 -and $focoPct -ge 95
    if ($limpa -or $tent -eq 1) { break }
    if ($temQuadros) { Avisar "A medida pegou uma travada ou o jogo fora da tela. Vou medir de novo, fique no jogo." }
  }
  $media = { param($a) if ($a.Count) { "{0:N1}" -f (($a | Measure-Object -Average).Average) } else { "?" } }
  return [pscustomobject]@{
    Fps = $kv["fps_avg"]; Low1 = $kv["fps_low1"]; Pior = $kv["worst_ms"]; Quadros = $kv["frames"]
    Erro = (Get-Content $out -ErrorAction SilentlyContinue | Where-Object { $_ -match "ERRO|erro|negado" }) -join " "
    Foco = [int]$focoPct; Repetida = ($tent -gt 0)
    Cpu = & $media $cpu; Mem = & $media $mem; Enc = & $media $enc; G3d = & $media $g3d
  }
}

function Linha([string]$rotulo, $m) {
  $rep = if ($m.Repetida) { " | medida repetida" } else { "" }
  Anotar ("{0}: FPS médio {1} | 1% low {2} | pior quadro {3} ms | jogo em foco {4}%{5} {6}" -f $rotulo, $m.Fps, $m.Low1, $m.Pior, $m.Foco, $rep, $m.Erro)
}

# ---------- início ----------
Clear-Host
Set-Content $Rel "Relatório do teste guiado — $Nome — $(Get-Date -Format 'dd/MM/yyyy HH:mm')"
Add-Content $Rel "Pasta: $PSScriptRoot"
Write-Host ""
Write-Host "  TESTE GUIADO DO $($Nome.ToUpper())" -ForegroundColor Cyan
Write-Host "  Vou fazendo tudo sozinho e te chamo quando precisar. São 11 etapas, ~30 min."
Write-Host "  No fim sai um arquivo só para mandar: $Rel"
Write-Host ""
Read-Host "  Aperte Enter para começar"

# 1. Diagnóstico
Titulo "1/11 Diagnóstico (automático)"
Fechar-Gravador   # o da bandeja segura o F8 e o diagnóstico acusaria a hotkey
$diag = ("" | & $Rec --diagnose 2>&1 | Where-Object { $_ -notmatch "^\d{4}-\d\d-\d\dT" } | Out-String).Trim()
Anotar $diag
$diagOk = $diag -match "Encode:\s+OK"
if (-not $diagOk) { Avisar "O diagnóstico falhou. Vou seguir, mas a gravação provavelmente não vai funcionar." }

# 2. Gravação rápida da tela, sem jogo
Titulo "2/11 Gravar a tela por 12 s e salvar (automático)"
Fechar-Gravador
if (-not (Subir-Gravador)) { Anotar "ERRO: o gravador não subiu na bandeja." }
else {
  $antes = Campo (Estado) "Último"
  Rec @("--record-screen") | Out-Null
  Write-Host "  Gravando a tela... (pode mexer no PC normalmente)"
  Start-Sleep 12
  $st = Estado; Anotar ("Durante: " + (Campo $st "Estado") + " | " + (Campo $st "Vídeo"))
  Rec @("--save") | Out-Null
  $clipe = ""
  for ($i = 0; $i -lt 20; $i++) { Start-Sleep 1; $u = Campo (Estado) "Último"; if ($u -and $u -ne $antes) { $clipe = $u; break } }
  if ($clipe -and (Test-Path $clipe)) { Anotar ("Clipe salvo: {0} ({1:N1} MB)" -f $clipe, ((Get-Item $clipe).Length / 1MB)) }
  else { Anotar "ERRO: o clipe da tela não apareceu em 20 s." }
  Rec @("--stop") | Out-Null
}
Fechar-Gravador

# 3. Jogo: FPS sem e com o gravador
Titulo "3/11 Jogo: FPS sem e com o $Nome"
Write-Host "  Abra o Valorant e entre no Campo de Treino (The Range)."
Write-Host "  Antes: nas configurações de vídeo do jogo, DESLIGUE o limite de FPS (só durante o teste)."
Write-Host "  Fique no campo de treino (não no menu nem carregando) e não saia do jogo até eu falar."
Write-Host "  Deixe o jogo NA TELA (pode ficar parado ou atirando). Eu detecto sozinho"
Write-Host "  e aviso por som quando mudar de fase. Não precisa voltar aqui até eu falar."
Write-Host "  (Se for outro jogo, só deixe ele em tela cheia na frente.)"
$jogoPid = 0
while (-not $jogoPid) {
  Start-Sleep 1
  $fg = Pid-Em-Foco
  $proc = Get-Process -Id $fg -ErrorAction SilentlyContinue
  if (-not $proc) { continue }
  if ($proc.ProcessName -eq $Jogo -or ((Tela-Cheia) -and $proc.ProcessName -notin @("explorer", "WindowsTerminal", "powershell", "pwsh", "cmd", "conhost", "OpenConsole", "Taskmgr", $Nome))) {
    $ok = $true
    for ($i = 0; $i -lt 5; $i++) { Start-Sleep 1; if ((Pid-Em-Foco) -ne $fg) { $ok = $false; break } }
    if ($ok) { $jogoPid = $fg; $jogoNome = $proc.ProcessName }
  }
}
Anotar "Jogo: $jogoNome (pid $jogoPid), tela cheia: $(Tela-Cheia)"
Avisar "Achei o jogo. Medindo sem o gravador por $Segundos segundos. Continue jogando."
$sem = Medir $jogoPid $Segundos
Linha "SEM gravador (1)" $sem

Avisar "Agora vou ligar o gravador. Continue no jogo."
[void](Subir-Gravador)
$recPid = (Get-Process ([IO.Path]::GetFileNameWithoutExtension($Rec)) -ErrorAction SilentlyContinue | Select-Object -First 1).Id
$gravando = $false
for ($i = 0; $i -lt 60; $i++) { Start-Sleep 1; if ((Campo (Estado) "Estado") -like "gravando*") { $gravando = $true; break } }
$st = Estado
Anotar ("Detecção automática: " + $(if ($gravando) { "OK em ~$i s" } else { "NÃO começou a gravar em 60 s" }) + " | " + (Campo $st "Estado") + " | " + (Campo $st "Vídeo"))
if (-not $gravando) {
  Avisar "O gravador não detectou o jogo. Vou forçar a gravação da tela para medir mesmo assim."
  Rec @("--record-screen") | Out-Null; Start-Sleep 3
}
Avisar "Gravando. Medindo com o gravador por $Segundos segundos."
$com = Medir $jogoPid $Segundos $recPid
Linha "COM gravador" $com
Anotar ("Gravador: CPU {0}% | memória {1} MB | GPU encode {2}% | GPU 3D {3}%" -f $com.Cpu, $com.Mem, $com.Enc, $com.G3d)

$antes = Campo (Estado) "Último"
Avisar "Agora aperte F8 no jogo para salvar um clipe."
$clipeJogo = ""
for ($i = 0; $i -lt 90; $i++) { Start-Sleep 1; $u = Campo (Estado) "Último"; if ($u -and $u -ne $antes) { $clipeJogo = $u; break } }
if ($clipeJogo) { Anotar "F8: OK, salvou em ~$i s" }
else {
  Anotar "F8: não salvou em 90 s; salvando pelo comando"
  Rec @("--save") | Out-Null
  for ($i = 0; $i -lt 20; $i++) { Start-Sleep 1; $u = Campo (Estado) "Último"; if ($u -and $u -ne $antes) { $clipeJogo = $u; break } }
}
$st = Estado; Anotar $st
if ($clipeJogo -and (Test-Path $clipeJogo)) { Anotar ("Clipe do jogo: {0} ({1:N1} MB)" -f $clipeJogo, ((Get-Item $clipeJogo).Length / 1MB)) }

# De novo sem o gravador: a perda é contra a média das duas medidas sem, que absorve a variação do próprio jogo.
Avisar "Vou desligar o gravador e medir sem ele mais uma vez. Continue no jogo."
Fechar-Gravador
Start-Sleep 3
$sem2 = Medir $jogoPid $Segundos
Linha "SEM gravador (2)" $sem2
$base = @(@($sem, $sem2) | Where-Object { $_.Fps } | ForEach-Object { [double]$_.Fps })
if ($base.Count -and $com.Fps) {
  $b = ($base | Measure-Object -Average).Average
  Anotar ("Perda de FPS: {0:N1}% (com {1} vs média sem {2:N1})" -f (100 * (1 - [double]$com.Fps / $b)), $com.Fps, $b)
}
Avisar "Pronto, pode sair do jogo. Volte para a janela do teste."
Write-Host ""
Read-Host "  Quando estiver aqui, aperte Enter"
[void](SimNao "Teve travada, tela preta ou aviso do Vanguard durante o teste?")
[void](SimNao "Você ouviu o bipe quando apertou F8?")

# 4. O clipe: som e imagem (só você consegue julgar)
Titulo "4/11 Conferir o clipe do jogo"
if ($clipeJogo -and (Test-Path $clipeJogo)) {
  Write-Host "  Vou abrir o clipe no player do Windows. Assista uns segundos e feche."
  Start-Process $clipeJogo
  Read-Host "  Aperte Enter depois de fechar o vídeo"
  [void](SimNao "O vídeo tem a imagem do jogo (sem tela preta)?")
  [void](SimNao "Mostra só o jogo (nada da área de trabalho, terminal ou outras janelas)?")
  [void](SimNao "Tem o som do jogo?")
  [void](SimNao "Tem o som do seu microfone? (se você falou algo)")
  [void](SimNao "Som e imagem estão sincronizados?")
  [void](SimNao "Tem uns 60 s de duração?")
} else { Anotar "Sem clipe do jogo para conferir." }

# 5. Player do app (automático)
Titulo "5/11 Player do app (automático, ~50 s)"
$alvo = if ($clipeJogo -and (Test-Path $clipeJogo)) { $clipeJogo } else { $clipe }
if ($alvo) {
  Write-Host "  Vai abrir o player, tocar em janela, depois em tela cheia, e pular pelo vídeo."
  Write-Host "  Não mexa no mouse/teclado até ele fechar sozinho. Só observe."
  Start-Sleep 3
  $pr = Join-Path $Tmp "player.txt"; Remove-Item $pr -ErrorAction SilentlyContinue
  $p = Start-Process $App -ArgumentList "--player-test", "`"$alvo`"", "--auto", "`"$pr`"" -PassThru
  if (-not $p.WaitForExit(120000)) { Stop-Process $p -Force; Anotar "ERRO: o player não fechou em 2 min." }
  if (Test-Path $pr) { Anotar (Get-Content $pr -Raw) } else { Anotar "ERRO: o player não gerou o relatório." }
  [void](SimNao "O vídeo no player pareceu liso (sem engasgos)?")
  [void](SimNao "As cores pareceram iguais às do player do Windows (nem lavadas nem escuras)?")
} else { Anotar "Sem clipe para testar o player." }

# 6. Biblioteca do app
Titulo "6/11 Biblioteca do app"
[void](Subir-Gravador)
Write-Host "  Vou abrir o app com o gravador ligado. Deixe a janela aberta até eu pedir para fechar."
$p = Start-Process $App -PassThru
Start-Sleep 4
[void](SimNao "Aparecem os clipes deste teste, com miniatura, duração, jogo, tamanho e 'há N min'?")
[void](SimNao "No card do gravador (embaixo, na barra da esquerda) está 'Esperando jogo' com F8?")
[void](SimNao "Clicar num jogo no menu da esquerda mostra só os clipes dele?")
[void](SimNao "Digitar o nome do jogo na busca filtra a lista? (apague a busca depois)")
[void](SimNao "A ordenação (botão 'Mais novos') troca a ordem dos clipes?")
Write-Host "  Passe o mouse num clipe e clique na estrela."
[void](SimNao "Ele aparece em Favoritos, com a contagem certa?")
Write-Host "  Agora vou gravar a tela por 6 s e salvar. Olhe a biblioteca."
$antes = Campo (Estado) "Último"
Rec @("--record-screen") | Out-Null; Start-Sleep 6; Rec @("--save") | Out-Null
$clipe6 = ""
for ($i = 0; $i -lt 20; $i++) { Start-Sleep 1; $u = Campo (Estado) "Último"; if ($u -and $u -ne $antes) { $clipe6 = $u; break } }
Rec @("--stop") | Out-Null
Start-Sleep 2
[void](SimNao "O clipe novo apareceu sozinho no topo da lista, como 'agora'?")
Write-Host "  Vou fechar o gravador."
Fechar-Gravador
Start-Sleep 2
[void](SimNao "O card do gravador mudou para 'Gravador fechado' com o botão 'Iniciar gravador'?")
Write-Host "  Clique em 'Iniciar gravador'."
[void](SimNao "Voltou para 'Esperando jogo' em alguns segundos?")
[void](Perguntar "O que achou do visual da biblioteca? O que mudaria? (Enter para pular)")
Write-Host "  Pode fechar o app."
$p.WaitForExit()
Fechar-Gravador

# 7. Tela do clipe (editor)
Titulo "7/11 Tela do clipe e recorte"
Write-Host "  Vou abrir o app de novo (sem o gravador)."
$p = Start-Process $App -PassThru
Start-Sleep 4
Write-Host "  Clique no clipe do jogo (o de ~60 s)."
[void](SimNao "Abriu a tela do clipe, com o vídeo grande tocando sozinho e com som?")
[void](SimNao "O som está sincronizado com a imagem?")
Write-Host "  Na cápsula de vidro sobre o vídeo, clique em Jogo e depois em Microfone (e de novo para religar)."
[void](SimNao "Cada botão desliga e religa só o som dele?")
Write-Host "  Na faixa de miniaturas, arraste a alça branca da esquerda até uns 5 s e a da direita até uns 15 s."
[void](SimNao "As alças se mexem e o vídeo mostra na hora o quadro de onde a alça está?")
Write-Host "  Aperte Espaço para tocar e espere chegar no fim do recorte."
[void](SimNao "Tocou só o pedaço recortado e voltou sozinho para o início dele?")
Write-Host "  Clique em pontos diferentes da faixa e arraste o cursor branco."
[void](SimNao "O vídeo pula na hora para o ponto clicado (sem demora perceptível)?")
Write-Host "  No painel Publicar (direita), clique no campo Título, digite um título e aperte Enter."
Write-Host "  Depois aperte a seta para a direita e a seta para a esquerda do teclado."
[void](SimNao "As setas trocaram de clipe e, ao voltar, o título e o recorte continuavam lá?")
Write-Host "  Aperte F (tela cheia) e depois Esc."
[void](SimNao "Foi para tela cheia e voltou com o Esc?")
Write-Host "  Volte para a biblioteca pelo botão ‹ no canto de cima à esquerda."
[void](SimNao "O card desse clipe na biblioteca mostra o título novo?")
if ($clipe6) {
  Write-Host "  Abra o clipe curto (~6 s) da etapa anterior, aperte Del e confirme em Apagar."
  [void](SimNao "Pediu confirmação e, depois de confirmar, o clipe sumiu da lista?")
  Start-Sleep 1
  Anotar ("Arquivo do clipe apagado: " + $(if (Test-Path $clipe6) { "NÃO, ainda está no disco: $clipe6" } else { "OK, saiu do disco" }))
}
[void](Perguntar "O que achou da tela do clipe? O que mudaria? (Enter para pular)")
Write-Host "  Pode fechar o app."
$p.WaitForExit()

# 8. Publicar pelo app
Titulo "8/11 Publicar pelo app"
if (SimNao "Quer testar publicar pelo app agora? (login com Discord na 1ª vez, ~3 min)") {
  $p = Start-Process $App -PassThru
  Start-Sleep 4
  Write-Host "  Abra o clipe do jogo, deixe um recorte de uns 10 s, escolha 'Com o link' em 'Quem pode ver'"
  Write-Host "  e clique em 'Publicar e copiar link'."
  Write-Host "  Na primeira vez abre o navegador: entre com o Discord."
  [void](SimNao "Depois do login a aba disse 'Pronto, você entrou' e o app seguiu sozinho?")
  [void](SimNao "Apareceu 'Preparando o vídeo' e depois 'Enviando X de Y' com a barra andando?")
  [void](SimNao "No fim apareceu 'Link copiado', o painel virou 'Publicado' e o botão virou 'Copiar link'?")
  Write-Host "  Cole o link (Ctrl+V) num chat do Discord."
  [void](SimNao "O embed tocou só o trecho recortado, com som?")
  Write-Host "  No painel, em 'Quem pode ver', clique em 'Só eu' e abra o link numa aba anônima."
  [void](SimNao "Na aba anônima o clipe não abriu?")
  Write-Host "  Volte para 'Com o link'. Abra outro clipe, clique em Publicar e logo depois em Cancelar."
  [void](SimNao "Cancelou e o botão Publicar voltou?")
  Write-Host "  No clipe publicado, clique em 'Apagar do servidor' e confirme."
  [void](SimNao "O link parou de abrir e o clipe continua na biblioteca?")
  [void](Perguntar "O que achou do publicar? O que mudaria? (Enter para pular)")
  Write-Host "  Pode fechar o app."
  $p.WaitForExit()
} else { Anotar "Publicar: pulado." }

# 9. Configurações
Titulo "9/11 Configurações"
$p = Start-Process $App -PassThru
Start-Sleep 4
Write-Host "  Clique em Configurações (menu da esquerda, embaixo)."
[void](SimNao "A tela mostra Gravação, Áudio, Armazenamento, Jogos, Sistema e Conta?")
Write-Host "  Mude 'Guardar os últimos' para 60 s."
[void](SimNao "O card do gravador passou a dizer 'salva os últimos 60 s'?")
Write-Host "  Em Conta: clique em Sair e depois em 'Entrar com o Discord'."
[void](SimNao "Saiu e entrou de novo, mostrando 'Conectado como <seu nome>'?")
Write-Host "  Volte 'Guardar os últimos' para o valor de antes."
[void](Perguntar "Algo faltando nas Configurações? (Enter para pular)")
Write-Host "  Pode fechar o app."
$p.WaitForExit()

# 10. Servidor
Titulo "10/11 Site ($Site)"
if (SimNao "Quer testar o site agora? (login com Discord e envio de um clipe, ~3 min)") {
  $h264 = ""
  $ffm = Get-Command ffmpeg -ErrorAction SilentlyContinue
  if ($alvo -and $ffm) {
    $h264 = Join-Path $Desk "$Nome-teste-h264.mp4"
    Write-Host "  Convertendo o clipe para H.264 (o Discord não toca HEVC)..."
    & $ffm.Source -hide_banner -loglevel error -y -i $alvo -c:v libx264 -preset veryfast -crf 23 -c:a aac -movflags +faststart $h264
    if (Test-Path $h264) { Write-Host "  Pronto: $h264" } else { $h264 = "" }
  }
  Start-Process $Site
  Write-Host "  1. Clique em 'Entrar com Discord' e autorize."
  [void](SimNao "Entrou e apareceu 'Olá, <seu nome>' com Admin e Enviar vídeo no topo?")
  if ($h264) { Write-Host "  2. 'Enviar vídeo' e escolha o arquivo $h264 (está na Área de Trabalho)." }
  else { Write-Host "  2. 'Enviar vídeo' e escolha um MP4 H.264 (vídeo do celular serve)." }
  [void](SimNao "O envio terminou e a página do clipe tocou o vídeo?")
  $link = Perguntar "Cole aqui o link do clipe (botão 'Copiar link')"
  Write-Host "  3. Cole esse link num chat do Discord."
  [void](SimNao "O embed apareceu e o vídeo toca dentro do Discord?")
  Write-Host "  4. Abra o link numa aba anônima (Ctrl+Shift+N)."
  [void](SimNao "Tocou sem estar logado?")
  Write-Host "  5. No Admin: Novo convite (Usos 1, Dias 7) -> Gerar."
  [void](SimNao "Apareceu um link .../i/<código>?")
} else { Anotar "Site: pulado." }

# 11. Visual novo (Fase 5)
Titulo "11/11 Visual novo do $Nome (Fase 5)"
# CPU média do processo em `secs` segundos (% de uma máquina inteira, como o Gerenciador de Tarefas).
function Cpu-Do([System.Diagnostics.Process]$proc, [int]$secs) {
  $proc.Refresh(); $t0 = $proc.TotalProcessorTime; $w0 = Get-Date
  Start-Sleep $secs
  $proc.Refresh(); $dt = ($proc.TotalProcessorTime - $t0).TotalSeconds; $dw = ((Get-Date) - $w0).TotalSeconds
  [math]::Round(100 * $dt / ($dw * [Environment]::ProcessorCount), 1)
}
[void](Subir-Gravador)
$p = Start-Process $App -PassThru
Start-Sleep 5
Write-Host "  Migração (só se você já usava o Clipe antes):"
[void](SimNao "Os clipes antigos do Clipe aparecem, com miniatura, favoritos e 'Publicados'?")
[void](SimNao "Publicar (etapa 8) funcionou sem pedir login de novo? (responda n se pediu)")
Write-Host "  Vidro: deixe outra janela colorida atrás do app."
[void](SimNao "Atrás da janela do app aparece o que está atrás, desfocado?")
Write-Host "  Arraste a janela pelo título 'Biblioteca' e redimensione pelo canto."
[void](SimNao "Arrastar e redimensionar ficou liso, sem engasgo visível?")
Write-Host "  Não mexa em nada por 15 s (medindo a CPU do app parado)..."
Anotar "CPU do app parado (15 s): $(Cpu-Do $p 15)%"
Write-Host "  Abra um clipe e deixe tocando. Não mexa no mouse por 15 s (medindo)..."
Read-Host "  Aperte Enter quando o clipe estiver tocando"
Anotar "CPU do app tocando um clipe (15 s): $(Cpu-Do $p 15)%"
[void](SimNao "A cápsula de controles sobre o vídeo parece vidro (o vídeo aparece desfocado atrás dela)?")
Write-Host "  Aperte F (tela cheia) e não mexa o mouse por 3 s."
[void](SimNao "Em tela cheia a cápsula some sozinha e volta ao mexer o mouse?")
Write-Host "  Aperte Esc e depois Del (sem confirmar)."
[void](SimNao "O aviso 'Apagar este clipe?' aparece em vidro, só o botão Apagar em vermelho? (clique Cancelar)")
Write-Host "  Tema: Configurações do Windows > Personalização > Cores > 'Escolha o modo': Claro."
[void](SimNao "O app ficou claro na hora, sem reiniciar? (volte para o Escuro depois)")
Write-Host "  Na mesma tela, desligue 'Efeitos de transparência'. Volte para o app (clique nele)."
[void](SimNao "O app ficou sólido (sem desfoque) e legível? (religue a transparência depois)")
Write-Host "  Bandeja: olhe o ícone do $Nome perto do relógio."
[void](SimNao "O ícone é um anel com um ponto no meio, na cor da barra (branco na escura, preto na clara)?")
Write-Host "  Clique com o botão direito no ícone."
[void](SimNao "O menu abriu com cantos redondos e fundo de vidro, junto do cursor?")
[void](SimNao "As setas ↑ ↓ andam pelos itens, Enter escolhe e Esc (ou clicar fora) fecha?")
Write-Host "  No menu, escolha 'Gravar a tela agora' e olhe o ícone."
[void](SimNao "O ponto do ícone ficou vermelho (e a primeira linha do menu também mostra o ponto vermelho)?")
Write-Host "  No menu, pare a gravação."
[void](SimNao "O ícone novo (lente) aparece no $Nome.exe no Explorer, na barra de tarefas e no Alt+Tab?")
Write-Host "  Site: abra um link publicado (o da etapa 8, se publicou)."
[void](SimNao "A página tem o visual novo (cabeçalho de vidro, botão de tocar redondo no meio do vídeo)?")
[void](SimNao "Colado no Discord, a imagem do link tem o botão de vidro no meio e a lente no canto?")
[void](Perguntar "O que achou do visual novo (app, bandeja, site)? O que mudaria? (Enter para pular)")
Write-Host "  Pode fechar o app."
$p.WaitForExit()
Fechar-Gravador

# ---------- fim ----------
Titulo "Logs"
$logDir = Join-Path $env:LOCALAPPDATA "$Nome\data\logs"
$log = Get-ChildItem $logDir -Filter "recorder*.log" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($log) { Add-Content $Rel "($($log.FullName), últimas 300 linhas)"; Add-Content $Rel (Get-Content $log.FullName -Tail 300) }
$cfg = Join-Path $env:APPDATA "$Nome\config\config.toml"
if (Test-Path $cfg) { Add-Content $Rel "`r`n--- config.toml ---"; Add-Content $Rel (Get-Content $cfg) }

Write-Host ""
Write-Host "  TERMINOU! Obrigado." -ForegroundColor Green
Write-Host "  Mande este arquivo para quem pediu o teste (vou abrir a pasta com ele selecionado):"
Write-Host "  $Rel" -ForegroundColor Green
Start-Process explorer.exe "/select,`"$Rel`""
Read-Host "  Enter para fechar"
