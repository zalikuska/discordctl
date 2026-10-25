# E2E-проверка шима на ПОДДЕЛЬНОЙ установке Discord во временной папке.
# Настоящий Discord не затрагивается: всё идёт через DISCORDCTL_* переменные окружения.
# Запуск: cargo build --release; powershell -File tests\e2e-shim.ps1

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $repo 'target\release'
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$tmp = Join-Path $env:TEMP "dctl-e2e-$id"
$root = Join-Path $tmp 'Discord'
$env:DISCORDCTL_HOME = Join-Path $tmp 'home'
$env:DISCORDCTL_DISCORD_ROOT = $root
$env:DISCORDCTL_PIPE = "\\.\pipe\discordctl-e2e-$id"
$fakeLog = Join-Path $root 'fake.log'
$script:fail = 0
$script:total = 0

# Работаем с копиями бинарников: cargo не блокируется, антивирус успевает проверить новые файлы.
$run = Join-Path $tmp 'bin'
New-Item -ItemType Directory -Force $run | Out-Null
foreach ($n in 'discordctl', 'discordctl-shim', 'discordctl-daemon', 'fake-discord') { Copy-Item (Join-Path $bin "$n.exe") $run }
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
    Write-Host "== setup: fake Discord in $root"
    New-Item -ItemType Directory -Force "$root\app-1.0.1", "$root\app-1.0.2" | Out-Null
    $fake = Join-Path $bin 'fake-discord.exe'
    Copy-Item $fake "$root\Update.exe"
    Copy-Item $fake "$root\Discord.exe"          # корневая заглушка
    Copy-Item $fake "$root\app-1.0.1\Discord.exe"
    Copy-Item $fake "$root\app-1.0.2\Discord.exe"

    Write-Host "== install on both targets"
    $s = Shim
    Check 'before install: both not_installed' ($s.update -eq 'not_installed' -and $s.stub -eq 'not_installed')
    & $ctl install --root $root --no-link-hook | Out-Null
    $s = Shim
    Check 'after install: Update.exe installed' ($s.update -eq 'installed')
    Check 'after install: Discord.exe stub installed' ($s.stub -eq 'installed')
    Check 'Update.real.exe and Discord.real.exe exist' ((Test-Path "$root\Update.real.exe") -and (Test-Path "$root\Discord.real.exe"))
    Check 'both entry points are now the shim' (((Get-Item "$root\Update.exe").Length -ne (Get-Item $fake).Length) -and ((Get-Item "$root\Discord.exe").Length -ne (Get-Item $fake).Length))

    Write-Host "== taskbar / autostart style: Update.exe --processStart"
    & "$root\Update.exe" --processStart Discord.exe
    Check 'Discord started with CDP pipe' (Wait-Until { (Count 'discord CDP') -eq 1 })
    Check 'launch was NOT forwarded to Update.real.exe' ((Count 'update\.real args') -eq 0)
    $st = & $ctl status --json | ConvertFrom-Json
    Check 'daemon reports managed Discord on the newest app-* (1.0.2)' ($st.daemon_running -and $st.daemon.managed_discord.pid -gt 0 -and $st.daemon.managed_discord.app_dir -like '*app-1.0.2')

    Write-Host "== deep link goes to the running instance"
    & "$root\Update.exe" --processStart Discord.exe --process-start-args '--url -- discord://-/channels/1/2'
    Check 'link forwarded as plain launch with args' (Wait-Until { (Log) -match 'PLAIN.*discord://-/channels/1/2' })

    Write-Host "== dangerous args are refused and fall back (Update.exe)"
    & "$root\Update.exe" --processStart Discord.exe --process-start-args '--remote-debugging-port=9222'
    Check 'fell back to Update.real.exe' (Wait-Until { (Log) -match 'update\.real args=\["--processStart"' })
    Check 'debug port never reached a CDP launch' (-not ((Log) -match 'CDP.*9222'))

    Write-Host "== other Update.exe commands pass through"
    & "$root\Update.exe" --update http://example.invalid
    Check 'passed to Update.real.exe' (Wait-Until { (Log) -match 'update\.real args=\["--update"' })

    Write-Host "== root stub while managed Discord runs"
    $plain = Count 'PLAIN'
    & "$root\Discord.exe"
    Check 'stub launch is forwarded to the running instance (no new CDP, no stub.real)' ((Wait-Until { (Count 'PLAIN') -gt $plain }) -and (Count 'discord CDP') -eq 1 -and (Count 'stub\.real') -eq 0)

    # Каждый запуск fake-discord затирает заглушку через ~0.8 с. Даём демону вернуть шим до остановки.
    $repairLog = Join-Path $env:DISCORDCTL_HOME 'daemon.log'
    Check 'watcher restored the stub shim right after Squirrel overwrote it' (Wait-Until { ((Repairs) -ge 1) -and ((Shim).stub -eq 'installed') } 8)
    Start-Sleep -Milliseconds 2500

    Write-Host "== daemon stop protection"
    $stop = & $ctl daemon stop 2>&1 | Out-String
    Check 'stop refused while managed Discord runs' ($LASTEXITCODE -ne 0 -and $stop -match 'force')
    Check 'daemon still up' ((& $ctl status --json | ConvertFrom-Json).daemon_running)
    Check 'daemon stop --force works' (StopDaemon)
    Check 'managed Discord exits when the pipe closes' (Wait-Until { (Log) -match 'pipe closed, exiting' })

    # Реальный Squirrel затирает корневую заглушку при каждом запуске Discord (fake-discord делает то же).
    # Демон должен вернуть шим сразу, а не по 30-секундному таймеру, иначе следующий запуск пойдёт мимо шима.
    Write-Host "== desktop shortcut style: Discord.exe --processStart Discord.exe"
    $repairsBefore = Repairs
    & "$root\Discord.exe" --processStart Discord.exe
    Check 'desktop-style launch is managed' (Wait-Until { (Count 'discord CDP') -eq 2 })
    Check 'it did not fall through to the real stub' ((Count 'stub\.real') -eq 0)
    Check 'Squirrel overwrote the stub and the watcher restored the shim at once' (Wait-Until { ((Repairs) -gt $repairsBefore) -and ((Shim).stub -eq 'installed') } 8)
    Start-Sleep -Milliseconds 400
    KillManaged

    Write-Host "== Start Menu shortcut style: bare Discord.exe, right after the previous Discord closed"
    & "$root\Discord.exe"
    Check 'start-menu-style launch is managed' (Wait-Until { (Count 'discord CDP') -eq 3 })
    Check 'it did not fall through to the real stub' ((Count 'stub\.real') -eq 0)
    Start-Sleep -Milliseconds 2000
    KillManaged

    Write-Host "== third rapid launch (desktop style again)"
    & "$root\Discord.exe" --processStart Discord.exe
    Check 'third launch is managed too' (Wait-Until { (Count 'discord CDP') -eq 4 })
    Start-Sleep -Milliseconds 2000
    KillManaged
    Check 'stub shim is back in place after the cycle' ((Shim).stub -eq 'installed')

    Write-Host "== dangerous / unknown args on the stub fall back"
    & "$root\Discord.exe" --remote-debugging-port=9222
    Check 'stub: debug port forwarded to Discord.real.exe' (Wait-Until { (Log) -match 'stub\.real args=\["--remote-debugging-port=9222"\]' })
    Check 'stub: debug port never reached a CDP launch' (-not ((Log) -match 'CDP.*9222'))
    & "$root\Discord.exe" --weird-flag
    Check 'stub: unknown flag forwarded to Discord.real.exe' (Wait-Until { (Log) -match 'stub\.real args=\["--weird-flag"\]' })

    Write-Host "== fallback when the daemon cannot start"
    Start-Sleep -Milliseconds 2000   # даём watcher'у вернуть заглушку после предыдущих запусков
    Check 'stub shim intact before the daemon is stopped' ((Shim).stub -eq 'installed')
    $null = StopDaemon
    $env:FAKE_NO_RESTORE = '1'       # дальше демона нет, fake-discord не затирает заглушку
    $cfgPath = Join-Path $env:DISCORDCTL_HOME 'config.json'
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    $good = $cfg.daemon_exe
    $cfg.daemon_exe = "$tmp\does-not-exist.exe"
    $cfg | ConvertTo-Json | Set-Content $cfgPath
    $u = Count 'update\.real args=\["--processStart"'
    $b = Count 'stub\.real args=\[\]'
    & "$root\Update.exe" --processStart Discord.exe
    Check 'Update.exe shim forwarded instead of failing' (Wait-Until { (Count 'update\.real args=\["--processStart"') -gt $u })
    & "$root\Discord.exe"
    Check 'stub shim forwarded instead of failing' (Wait-Until { (Count 'stub\.real args=\[\]') -gt $b })
    $cfg.daemon_exe = $good
    $cfg | ConvertTo-Json | Set-Content $cfgPath

    Write-Host "== updater overwrites the shims, then repair"
    Copy-Item $fake "$root\Update.exe" -Force
    Copy-Item $fake "$root\Discord.exe" -Force
    $s = Shim
    Check 'detected: both need_repair' ($s.update -eq 'needs_repair' -and $s.stub -eq 'needs_repair')
    & $ctl doctor --fix | Out-Null
    $s = Shim
    Check 'repaired: both installed again' ($s.update -eq 'installed' -and $s.stub -eq 'installed')

    Write-Host "== uninstall"
    & $ctl uninstall | Out-Null
    $s = Shim
    Check 'both not_installed' ($s.update -eq 'not_installed' -and $s.stub -eq 'not_installed')
    Check '*.real.exe files are gone' (-not (Test-Path "$root\Update.real.exe") -and -not (Test-Path "$root\Discord.real.exe"))
    Check 'originals are back' (((Get-Item "$root\Update.exe").Length -eq (Get-Item $fake).Length) -and ((Get-Item "$root\Discord.exe").Length -eq (Get-Item $fake).Length))
}
finally {
    & $ctl daemon stop --force 2>$null | Out-Null
    Get-Process -Name 'Discord', 'discordctl-daemon' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$tmp*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    Write-Host "== fake.log"
    Log
    foreach ($n in 'daemon', 'shim') {
        $f = Join-Path $env:DISCORDCTL_HOME "$n.log"
        if (Test-Path $f) { Write-Host "== $n.log"; Get-Content $f }
    }
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
$expected = (Select-String -Path $PSCommandPath -Pattern "^\s*Check '").Count
if ($script:total -lt $expected) { Write-Host "only $script:total of $expected checks ran" -ForegroundColor Red; $script:fail++ }
if ($script:fail -gt 0) { Write-Host "$script:fail check(s) FAILED" -ForegroundColor Red; exit 1 }
Write-Host "ALL $script:total CHECKS PASSED" -ForegroundColor Green
