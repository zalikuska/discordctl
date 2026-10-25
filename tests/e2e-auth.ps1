# Проверка прав на живом демоне (нужен запущенный Discord с активным хуком). Ничего не отправляет:
# запись проверяется отказами, удалением несуществующего сообщения и --dry-run.
# Выдачу прав делаем сырым запросом в pipe, в обход терминального подтверждения CLI: его нельзя автоматизировать.
$ErrorActionPreference = 'Stop'
$d = Join-Path $PSScriptRoot '..\target\release\discordctl.exe'
$user = ($env:USERNAME -replace '[^A-Za-z0-9]', '_')

function Send-Owner($obj) {
    $p = New-Object IO.Pipes.NamedPipeClientStream('.', "discordctl-$user", 'InOut')
    $p.Connect(3000)
    $w = New-Object IO.StreamWriter($p, (New-Object Text.UTF8Encoding($false)))
    $w.WriteLine(($obj | ConvertTo-Json -Compress -Depth 6)); $w.Flush()
    $r = New-Object IO.StreamReader($p, (New-Object Text.UTF8Encoding($false)))
    $line = $r.ReadLine(); $p.Dispose()
    $line | ConvertFrom-Json
}
function As-App($token, [scriptblock]$body) {
    $old = $env:DISCORDCTL_TOKEN; $env:DISCORDCTL_TOKEN = $token
    try { & $body } finally { $env:DISCORDCTL_TOKEN = $old }
}
$fail = 0
function Check($name, [bool]$ok, $extra = '') { if ($ok) { "{0,-52} ok" -f $name } else { ("{0,-52} FAIL {1}" -f $name, $extra); $script:fail++ } }
function Run-Cli {
    # Ошибки CLI нормальны (отказы проверяем), поэтому stderr не должен прерывать скрипт.
    $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    $out = & $d @args 2>&1 | Out-String
    $code = $LASTEXITCODE; $ErrorActionPreference = $prev
    [pscustomobject]@{ ok = ($code -eq 0); text = $out.Trim() }
}

# Что есть у владельца: берём настоящие id.
$dms = (& $d chat --json) -join "`n" | ConvertFrom-Json
$group = $dms | Where-Object { $_.kind -eq 'group' } | Select-Object -First 1
$other = $dms | Where-Object { $_.id -ne $group.id } | Select-Object -First 1
$guild = ((& $d server --json) -join "`n" | ConvertFrom-Json) | Where-Object { $_.owner } | Select-Object -First 1
$gch = ((& $d channel list $guild.id --json) -join "`n" | ConvertFrom-Json) | Where-Object { $_.kind -eq 'text' } | Select-Object -First 1
"fixtures: group=$($group.id) other=$($other.id) guild=$($guild.id) channel=$($gch.id)"

$name = 'e2e-auth-test'
$g = Send-Owner @{ cmd = 'auth_grant'; name = $name; permissions = @{ list = @('guilds'); read = @("dm:$($group.id)"); write = @("dm:$($group.id)") } }
Check 'owner can grant' ($g.ok -and $g.data.token -like 'dctl_*')
$tok = $g.data.token

Check 'bad manifest is rejected' (-not (Send-Owner @{ cmd = 'auth_grant'; name = 'x'; permissions = @{ read = @('dm:SomeChat') } }).ok)

As-App $tok {
    $w = Run-Cli app whoami --json
    Check 'whoami says app' ($w.ok -and $w.text -match $name)
    Check 'list guilds allowed' (Run-Cli server --json).ok
    Check 'list dms denied (not granted)' (-not (Run-Cli chat --json).ok)
    Check 'read granted chat by id' (Run-Cli read $group.id -n 2 --json).ok
    $r = Run-Cli read "@$($group.name)" -n 2 --json
    Check 'read granted chat by NAME resolves to same id' $r.ok $r.text
    $r = Run-Cli read $other.id -n 1 --json
    Check 'read other chat denied' ((-not $r.ok) -and $r.text -match 'no permission')
    $r = Run-Cli read "$($guild.id)/$($gch.id)" -n 1 --json
    Check 'read server channel denied' (-not $r.ok)
    $r = Run-Cli read '@несуществующий-чат' --json
    Check 'unknown chat gives a generic error' ((-not $r.ok) -and $r.text -notmatch 'ambiguous')
    Check 'raw call denied' (-not (Run-Cli dev call selfcheck).ok)
    Check 'hook reload denied' (-not (Run-Cli dev reload).ok)
    Check 'daemon stop denied' (-not (Run-Cli daemon stop --force).ok)
    Check 'auth list denied' (-not (Run-Cli app list).ok)
    Check 'dry-run write needs no approval' (Run-Cli send "dm:$($group.id)" 'probe' --dry-run).ok
    Check 'write to other chat denied' (-not (Run-Cli send "dm:$($other.id)" 'probe' --dry-run).ok)
}
Check 'invalid token is not owner' (As-App 'dctl_deadbeef' { -not (Run-Cli server --json).ok })

# Подтверждение: вызов ждёт человека; отклоняем — ничего не уходит.
$job = Start-Job -ArgumentList $d, $tok, $group.id { param($d, $t, $id) $env:DISCORDCTL_TOKEN = $t; & $d message edit "dm:$id" 100000000000000000 'must never be sent' 2>&1 | Out-String }
$id = $null
for ($i = 0; $i -lt 40 -and -not $id; $i++) { Start-Sleep -Milliseconds 250; $p = (Send-Owner @{ cmd = 'approvals' }).data; if ($p) { $id = @($p)[0].id } }
Check 'call waits for approval' ($null -ne $id)
Check 'summary shows the text to the human' ($id -and (@($p)[0].summary -match 'must never be sent'))
if ($id) { Send-Owner @{ cmd = 'decide'; id = $id; allow = $false } | Out-Null }
$res = Receive-Job $job -Wait; Remove-Job $job
Check 'rejected call fails and says why' ($res -match 'rejected by the user')

# Подтверждённый вызов доходит до Discord: удаляем несуществующее сообщение, побочных эффектов нет.
$job = Start-Job -ArgumentList $d, $tok, $group.id { param($d, $t, $id) $env:DISCORDCTL_TOKEN = $t; & $d message delete "dm:$id" 100000000000000000 2>&1 | Out-String }
$id = $null
for ($i = 0; $i -lt 40 -and -not $id; $i++) { Start-Sleep -Milliseconds 250; $p = (Send-Owner @{ cmd = 'approvals' }).data; if ($p) { $id = @($p)[0].id } }
if ($id) { Send-Owner @{ cmd = 'decide'; id = $id; allow = $true } | Out-Null }
$res = Receive-Job $job -Wait; Remove-Job $job
Check 'allowed call reaches Discord (404, nothing deleted)' ($res -match 'HTTP 404')

# Голос: права по словам и по каналам. Живых входов в звонок нет: проверяем отказы, dry-run и отклонённое подтверждение.
$vch = ((& $d channel list $guild.id --json) -join "`n" | ConvertFrom-Json) | Where-Object { $_.kind -eq 'voice' } | Select-Object -First 1
if ($vch) {
    $vref = "$($guild.id)/$($vch.id)"
    $vname = 'e2e-voice-test'
    $vg = Send-Owner @{ cmd = 'auth_grant'; name = $vname; permissions = @{ voice = @('status', 'mute', 'join'); voice_channels = @("guild:$($guild.id)/channel:$($vch.id)") } }
    Check 'voice grant accepted' ($vg.ok)
    Check 'voice_channels without join is rejected' (-not (Send-Owner @{ cmd = 'auth_grant'; name = 'x'; permissions = @{ voice = @('mute'); voice_channels = @("guild:$($guild.id)") } }).ok)
    As-App $vg.data.token {
        Check 'voice status allowed' (Run-Cli voice status --json).ok
        $r = Run-Cli voice join "dm:$($group.id)" --dry-run
        Check 'join of a chat not in voice_channels denied' ((-not $r.ok) -and $r.text -match 'no permission')
        $r = Run-Cli voice join $vref --dry-run
        Check 'join of the allowed channel (dry run) passes' ($r.ok -and $r.text -match 'would_ring')
        Check 'leave denied (word not granted)' (-not (Run-Cli voice leave --dry-run).ok)
        Check 'deafen denied (word not granted)' (-not (Run-Cli voice deafen).ok)
    }
    $before = ((& $d voice status --json) -join "`n" | ConvertFrom-Json).self_mute
    $job = Start-Job -ArgumentList $d, $vg.data.token { param($d, $t) $env:DISCORDCTL_TOKEN = $t; & $d voice mute 2>&1 | Out-String }
    $vid = $null
    for ($i = 0; $i -lt 40 -and -not $vid; $i++) { Start-Sleep -Milliseconds 250; $p = (Send-Owner @{ cmd = 'approvals' }).data; if ($p) { $vid = @($p)[0].id } }
    Check 'mute waits for approval and says what it does' ($vid -and (@($p)[0].summary -match 'MUTE your microphone'))
    if ($vid) { Send-Owner @{ cmd = 'decide'; id = $vid; allow = $false } | Out-Null }
    $res = Receive-Job $job -Wait; Remove-Job $job
    Check 'rejected mute fails' ($res -match 'rejected by the user')
    Check 'rejected mute changed nothing' (((& $d voice status --json) -join "`n" | ConvertFrom-Json).self_mute -eq $before)
    Send-Owner @{ cmd = 'auth_revoke'; name = $vname } | Out-Null
} else { 'no voice channel in the fixture guild: voice checks skipped' }

$aud = (Send-Owner @{ cmd = 'auth_audit'; limit = 60 }).data
Check 'audit has allow, deny and reject entries' ((@($aud | Where-Object { $_.decision -eq 'allowed' }).Count -gt 0) -and (@($aud | Where-Object { $_.decision -eq 'denied' }).Count -gt 0))
Check 'audit never stores message text' (-not (($aud | ConvertTo-Json -Depth 4) -match 'must never be sent'))

Check 'revoke works' (Send-Owner @{ cmd = 'auth_revoke'; name = $name }).ok
Check 'revoked token stops at once' (As-App $tok { -not (Run-Cli server --json).ok })
if ($fail) { "$fail check(s) FAILED"; exit 1 } else { 'all checks ok' }
