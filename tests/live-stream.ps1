# Живая проверка трансляции с правами. ВХОДИТ В ГОЛОС И ТРАНСЛИРУЕТ ОКНО в -VoiceChannel (по умолчанию ничего не делает).
# Канал должен быть пустым и вашим. Микрофон и звук на время выключаются, потом возвращаются, окно -Window показывается зрителям канала.
param(
    [Parameter(Mandatory)][string]$VoiceChannel,   # сервер/канал, например "123/456"
    [Parameter(Mandatory)][string]$Window,         # часть названия безобидного окна
    [switch]$Yes
)
$ErrorActionPreference = 'Stop'
$d = Join-Path $PSScriptRoot '..\target\release\discordctl.exe'
$user = ($env:USERNAME -replace '[^A-Za-z0-9]', '_')
if (-not $Yes) { "Would join voice $VoiceChannel muted, stream the window matching '$Window' after an approval, stop, leave. Re-run with -Yes."; exit 0 }

function Send-Owner($obj) {
    $p = New-Object IO.Pipes.NamedPipeClientStream('.', "discordctl-$user", 'InOut'); $p.Connect(3000)
    $w = New-Object IO.StreamWriter($p, (New-Object Text.UTF8Encoding($false)))
    $w.WriteLine(($obj | ConvertTo-Json -Compress -Depth 6)); $w.Flush()
    $r = New-Object IO.StreamReader($p, (New-Object Text.UTF8Encoding($false))); $line = $r.ReadLine(); $p.Dispose()
    $line | ConvertFrom-Json
}
function Dctl { $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'; $o = & $d @args 2>&1 | Out-String; $c = $LASTEXITCODE; $ErrorActionPreference = $prev; [pscustomobject]@{ ok = ($c -eq 0); text = $o.Trim() } }
$fail = 0
function Check($name, [bool]$ok, $extra = '') { if ($ok) { "{0,-58} ok" -f $name } else { ("{0,-58} FAIL {1}" -f $name, $extra); $script:fail++ } }
function Wait-Approval { for ($i = 0; $i -lt 60; $i++) { Start-Sleep -Milliseconds 250; $p = (Send-Owner @{ cmd = 'approvals' }).data; if ($p) { return @($p)[0] } }; $null }

$plan = Dctl voice join $VoiceChannel --dry-run
if ($plan.text -notmatch '"participants_now": 0' -and $plan.text -notmatch 'participants_now.*0') { throw "voice channel is not empty or not found: $($plan.text)" }
$orig = (Dctl voice status --json).text | ConvertFrom-Json
$app = 'live-stream-test'
$g = Send-Owner @{ cmd = 'auth_grant'; name = $app; permissions = @{ stream = @('list', 'status', 'stop', 'window') } }
$tok = $g.data.token
try {
    Dctl voice mute | Out-Null; Dctl voice deafen | Out-Null
    Check 'owner joins the empty channel' (Dctl voice join $VoiceChannel).ok

    $env:DISCORDCTL_TOKEN = $tok
    Check 'app can list sources' (Dctl stream list).ok
    $scr = Dctl stream start 'screen:0:0' --dry-run
    Check 'app cannot start a whole screen (word not granted)' ((-not $scr.ok) -and $scr.text -match 'no permission')
    $env:DISCORDCTL_TOKEN = $null

    # Запуск приложением ждёт человека: сводка называет окно и канал.
    $job = Start-Job -ArgumentList $d, $tok, $Window { param($d, $t, $w) $env:DISCORDCTL_TOKEN = $t; & $d stream start $w 2>&1 | Out-String }
    $pending = Wait-Approval
    Check 'start waits for approval' ($null -ne $pending)
    Check 'summary names the window and warns about viewers' ($pending -and $pending.summary -match 'START STREAMING' -and $pending.summary -match [regex]::Escape($Window) -and $pending.summary -notmatch 'WITH SOUND')
    if ($pending) { Send-Owner @{ cmd = 'decide'; id = $pending.id; allow = $true } | Out-Null }
    $res = Receive-Job $job -Wait; Remove-Job $job
    Check 'approved start reaches the client' ($res -match 'streaming')
    Start-Sleep 3
    Check 'client reports the stream' ((Dctl stream status --json).text -match '"streaming": true')

    $env:DISCORDCTL_TOKEN = $tok
    Check 'app can stop its stream without approval' (Dctl stream stop).ok
    $env:DISCORDCTL_TOKEN = $null
    Check 'stream is gone' ((Dctl stream status --json).text -match '"streaming": false')
}
finally {
    $env:DISCORDCTL_TOKEN = $null
    Dctl stream stop | Out-Null
    Dctl voice leave | Out-Null
    if (-not $orig.self_deaf) { Dctl voice undeafen | Out-Null }
    if (-not $orig.self_mute) { Dctl voice unmute | Out-Null }
    Send-Owner @{ cmd = 'auth_revoke'; name = $app } | Out-Null
}
Check 'left the voice channel' ((Dctl voice status --json).text -match '"connected": false')
if ($fail) { "$fail check(s) FAILED"; exit 1 } else { 'all checks ok' }
