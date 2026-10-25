# E2E-проверка мастера setup, справки и подсказок на ПОДДЕЛЬНОЙ установке Discord во временной папке.
# Настоящий Discord и настоящий ключ discord:// не затрагиваются: DISCORDCTL_* и DISCORDCTL_HANDLER_KEY.
# Запуск: cargo build --release; powershell -File tests\e2e-setup.ps1

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $repo 'target\release'
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$tmp = Join-Path $env:TEMP "dctl-e2e-$id"
$root = Join-Path $tmp 'Discord'
$env:DISCORDCTL_HOME = Join-Path $tmp 'home'
$env:DISCORDCTL_DISCORD_ROOT = $root
$env:DISCORDCTL_PIPE = "\\.\pipe\discordctl-e2e-$id"
$env:DISCORDCTL_HANDLER_KEY = "Software\discordctl-test\e2e-$id"
$fakeLog = Join-Path $root 'fake.log'
$script:fail = 0
$script:total = 0

# Работаем с копиями бинарников: cargo не блокируется, антивирус успевает проверить новые файлы.
$run = Join-Path $tmp 'bin'
New-Item -ItemType Directory -Force $run | Out-Null
foreach ($n in 'discordctl', 'discordctl-shim', 'discordctl-daemon', 'discordctl-mcp', 'fake-discord') { Copy-Item (Join-Path $bin "$n.exe") $run }
$bin = $run
$ctl = Join-Path $run 'discordctl.exe'
$ready = $false
for ($i = 0; $i -lt 40 -and -not $ready; $i++) { try { $null = & $ctl --help 2>&1; $ready = ($LASTEXITCODE -eq 0) } catch { Start-Sleep -Milliseconds 500 } }
if (-not $ready) { Write-Host 'discordctl.exe will not start' -ForegroundColor Red; exit 2 }

function Check($name, $cond) {
    $script:total++
    if ($cond) { Write-Host "  PASS  $name" -ForegroundColor Green }
    else { Write-Host "  FAIL  $name" -ForegroundColor Red; $script:fail++ }
}
function Wait-Until($cond, $sec = 10) {
    $end = (Get-Date).AddSeconds($sec)
    while ((Get-Date) -lt $end) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 150 }
    return $false
}
function Log { if (Test-Path $fakeLog) { Get-Content $fakeLog -Raw } else { '' } }
function Count($pattern) { ([regex]::Matches((Log), $pattern)).Count }
function Shim { (& $ctl status --json | ConvertFrom-Json).shim }
function Repairs {
    # Сколько раз демон чинил заглушку (лог накопительный, поэтому сравниваем счётчики).
    if (Test-Path $repairLog) { ([regex]::Matches((Get-Content $repairLog -Raw), 'Discord\.exe was overwritten')).Count } else { 0 }
}
function KillManaged {
    # Имитирует закрытие Discord пользователем: демон остаётся жить.
    $m = (& $ctl status --json | ConvertFrom-Json).daemon.managed_discord
    if ($m) {
        Stop-Process -Id $m.pid -Force -ErrorAction SilentlyContinue
        $null = Wait-Until { -not ((& $ctl status --json | ConvertFrom-Json).daemon.managed_discord) }
    }
}
function StopDaemon { & $ctl daemon stop --force | Out-Null; Wait-Until { -not (& $ctl status --json | ConvertFrom-Json).daemon_running } }


try {
    New-Item -ItemType Directory -Force "$root\app-1.0.1" | Out-Null
    $fake = Join-Path $bin 'fake-discord.exe'
    Copy-Item $fake "$root\Update.exe"
    Copy-Item $fake "$root\Discord.exe"
    Copy-Item $fake "$root\app-1.0.1\Discord.exe"
    $env:DISCORDCTL_SETUP_WAIT = '4'
    $ESC = [char]27

    Write-Host "== help and completions"
    $help = & $ctl --help | Out-String
    Check 'help shows the grouped overview and targets' ($help -match 'Get started' -and $help -match 'Targets' -and $help -match 'mcp install')
    Check 'help has no color codes when piped' ($help -notmatch "$ESC")
    $comp = & $ctl completions powershell | Out-String
    Check 'PowerShell completions are generated' ($comp -match 'Register-ArgumentCompleter')
    Write-Host "== language"
    $prevEnc = [Console]::OutputEncoding
    [Console]::OutputEncoding = [Text.Encoding]::UTF8
    $ru = & $ctl --lang ru --help | Out-String
    Check 'Russian help with --lang ru' ($ru -match 'Использование' -and $ru -match 'С чего начать' -and $ru -notmatch 'Get started')
    $env:DISCORDCTL_LANG = 'ru'
    $viaEnv = & $ctl lang | Out-String
    Remove-Item Env:DISCORDCTL_LANG
    Check 'DISCORDCTL_LANG switches the language' ($viaEnv -match 'язык')
    & $ctl lang ru | Out-Null
    Check 'lang ru is remembered between runs' ((& $ctl lang | Out-String) -match 'язык: ru')
    Check 'English still wins with an explicit --lang en' ((& $ctl --lang en --help | Out-String) -match 'Get started')
    Check '--json output is not translated' ((& $ctl --json lang | ConvertFrom-Json).lang -eq 'ru')
    & $ctl lang fr 2>&1 | Out-Null
    Check 'an unknown language is refused' ($LASTEXITCODE -ne 0)
    & $ctl lang en | Out-Null
    Check 'back to English' ((& $ctl --help | Out-String) -match 'Get started')
    [Console]::OutputEncoding = $prevEnc

    $bad = & $ctl messag read 2>&1 | Out-String
    Check 'a typo gets a suggestion and a nonzero exit' ($LASTEXITCODE -ne 0 -and $bad -match 'similar|message')
    Check 'new verbs exist' ((& $ctl message --help | Out-String) -match 'react' -and (& $ctl voice --help | Out-String) -match 'unmute')

    Write-Host "== setup asks questions only in a terminal"
    $noTty = & $ctl setup 2>&1 | Out-String
    Check 'setup without --yes refuses to run piped' ($LASTEXITCODE -ne 0 -and $noTty -match 'interactive terminal')
    Check '...and changed nothing' ((Shim).update -eq 'not_installed')

    Write-Host "== setup --yes on the fake Discord"
    $out = & $ctl setup --yes --root $root --no-color 2>&1 | Out-String
    Check 'setup finished with exit code 0' ($LASTEXITCODE -eq 0)
    foreach ($h in '1. Checking', '2. Installing', '3. Starting Discord', '4. Health check', '5. AI agent') { Check "step printed: $h" ($out -match [regex]::Escape($h)) }
    Check 'no escape codes with --no-color' ($out -notmatch "$ESC")
    $s = Shim
    Check 'both shims installed by setup' ($s.update -eq 'installed' -and $s.stub -eq 'installed')
    Check 'setup left the PATH alone (custom data dir)' ($out -notmatch 'added to PATH')
    Check 'Discord was launched through the daemon' ((& $ctl status --json | ConvertFrom-Json).daemon.managed_discord.pid -gt 0)
    Check 'next steps are shown' ($out -match 'discordctl doctor')

    Write-Host "== agent commands"
    $m = & $ctl mcp install --agent print 2>&1 | Out-String
    Check 'mcp install needs a terminal to approve rights' ($LASTEXITCODE -ne 0 -and $m -match 'interactive terminal')
    $st = & $ctl mcp status --json | ConvertFrom-Json
    Check 'mcp status lists the three agents' (@($st.agents).Count -eq 3)
    $rm = & $ctl mcp remove --agent print --json | ConvertFrom-Json
    Check 'mcp remove is harmless when nothing is connected' (-not $rm.agents[0].config_removed)

    Write-Host "== watch"
    $watchOut = Join-Path $tmp 'watch.out'
    $w = Start-Process $ctl -ArgumentList 'watch', '--json', '-n', '2' -RedirectStandardOutput $watchOut -PassThru -WindowStyle Hidden
    # watch ждёт у демона; подождём, пока он займёт очередь, и подкинем события
    $null = Wait-Until { (& $ctl --json status | ConvertFrom-Json).daemon_running } 10
    Start-Sleep -Milliseconds 800
    $emitFile = Join-Path $tmp 'emit.json'
    '[{"kind":"message","ts":1000,"channel_id":"111111111111111111","guild_id":null,"message":{"content":"hi there"},"user":{"display_name":"alice"}},{"kind":"voice_join","ts":2000,"channel_id":"222222222222222222","guild_id":"3","user":{"display_name":"bob"}}]' | Set-Content $emitFile -Encoding ascii
    & $ctl dev emit "@$emitFile" | Out-Null
    Check 'watch --json prints each event as one JSON line and stops after -n' ((Wait-Until { $w.HasExited } 15) -and (@(Get-Content $watchOut | Where-Object { $_.Trim() }).Count -eq 2))
    $lines = @(Get-Content $watchOut | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
    Check 'the events arrive in order and intact' ($lines[0].kind -eq 'message' -and $lines[0].message.content -eq 'hi there' -and $lines[1].kind -eq 'voice_join')
    if (-not $w.HasExited) { Stop-Process -Id $w.Id -Force }

    Write-Host "== watch presence with a filter"
    $presOut = Join-Path $tmp 'presence.out'
    $pw = Start-Process $ctl -ArgumentList 'watch', '--json', '-n', '1', '--activity', 'minecraft', '--status', 'online' -RedirectStandardOutput $presOut -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 1500
    '[{"kind":"presence","ts":1,"channel_id":null,"guild_id":null,"user":{"id":"7","display_name":"chess player"},"status":"online","prev_status":"online","changes":[{"change":"started","type":"playing","name":"Chess"}]},{"kind":"presence","ts":2,"channel_id":null,"guild_id":null,"user":{"id":"8","display_name":"miner"},"status":"online","prev_status":"offline","changes":[{"change":"started","type":"playing","name":"Minecraft"}]}]' | Set-Content $emitFile -Encoding ascii
    & $ctl dev emit "@$emitFile" | Out-Null
    $got = (Wait-Until { $pw.HasExited } 15)
    $pl = @(Get-Content $presOut | Where-Object { $_.Trim() })
    Check 'watch --activity passes only the person who started that game' ($got -and $pl.Count -eq 1 -and ($pl[0] | ConvertFrom-Json).user.id -eq '8')
    if (-not $pw.HasExited) { Stop-Process -Id $pw.Id -Force }

    Write-Host "== uninstall"
    $null = & $ctl uninstall
    Check 'uninstall restores both files' ((Shim).update -eq 'not_installed' -and (Shim).stub -eq 'not_installed')
}
finally {
    & $ctl daemon stop --force 2>$null | Out-Null
    Get-Process -Name 'Discord', 'discordctl-daemon' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$tmp*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    Remove-Item "HKCU:\Software\discordctl-test" -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
$expected = (Select-String -Path $PSCommandPath -Pattern "^\s*Check '").Count
if ($script:total -lt $expected) { Write-Host "only $script:total of $expected checks ran" -ForegroundColor Red; $script:fail++ }
if ($script:fail -gt 0) { Write-Host "$script:fail check(s) FAILED" -ForegroundColor Red; exit 1 }
Write-Host "ALL $script:total CHECKS PASSED" -ForegroundColor Green
