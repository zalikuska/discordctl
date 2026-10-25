# E2E-проверка установки, doctor, хука discord:// и удаления на ПОДДЕЛЬНОЙ установке Discord во временной папке.
# Настоящий Discord и настоящий ключ discord:// не затрагиваются: DISCORDCTL_* и DISCORDCTL_HANDLER_KEY.
# Запуск: cargo build --release; powershell -File tests\e2e-ops.ps1

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
    Write-Host "== setup: fake Discord in $root, test registry key $($env:DISCORDCTL_HANDLER_KEY)"
    New-Item -ItemType Directory -Force "$root\app-1.0.1", "$root\app-1.0.2" | Out-Null
    $fake = Join-Path $bin 'fake-discord.exe'
    Copy-Item $fake "$root\Update.exe"
    Copy-Item $fake "$root\Discord.exe"
    Copy-Item $fake "$root\app-1.0.1\Discord.exe"
    Copy-Item $fake "$root\app-1.0.2\Discord.exe"
    $keyPs = "HKCU:\$($env:DISCORDCTL_HANDLER_KEY)\shell\open\command"
    $versioned = "`"$root\app-1.0.2\Discord.exe`" --url -- `"%1`""
    New-Item -Path $keyPs -Force | Out-Null
    Set-ItemProperty -Path $keyPs -Name '(default)' -Value $versioned
    function Handler { (Get-ItemProperty -Path $keyPs).'(default)' }
    $want = "`"$root\Discord.exe`" --url -- `"%1`""

    Write-Host "== doctor before install"
    $d0 = & $ctl doctor --json | ConvertFrom-Json
    Check 'doctor runs and says not installed' ($d0.checks | Where-Object { $_.name -eq 'installed binaries' -and $_.detail -eq 'not installed' })

    Write-Host "== one-command install"
    $out = & $ctl install --root $root --link-hook --json | ConvertFrom-Json
    $s = Shim
    Check 'both shims installed' ($s.update -eq 'installed' -and $s.stub -eq 'installed')
    Check 'discord:// handler now points at the stub' ((Handler) -eq $want)
    Check 'previous handler remembered for the record' ((Get-Content (Join-Path $env:DISCORDCTL_HOME 'config.json') -Raw) -match 'app-1\.0\.2')
    Check 'install is idempotent' ((& $ctl install --root $root --link-hook --json | ConvertFrom-Json).link_handler -eq 'already installed')

    Write-Host "== doctor after install"
    $d1 = & $ctl doctor --json | ConvertFrom-Json
    Check 'doctor: no FAIL on a healthy install' ($LASTEXITCODE -eq 0 -and -not ($d1.checks | Where-Object { $_.level -eq 'fail' }))
    Check 'doctor sees the shims and the link handler' ((($d1.checks | Where-Object { $_.name -eq 'Update.exe shim' }).level -eq 'ok') -and (($d1.checks | Where-Object { $_.name -eq 'discord:// links' }).level -eq 'ok'))

    Write-Host "== quarantine simulation: antivirus removes an installed binary"
    $binDir = Join-Path $env:DISCORDCTL_HOME 'bin'
    Remove-Item (Join-Path $binDir 'discordctl-shim.exe') -Force
    $d2 = & $ctl doctor --json | ConvertFrom-Json
    Check 'doctor reports the missing file as FAIL with the antivirus hint' (($LASTEXITCODE -eq 1) -and (($d2.checks | Where-Object { $_.name -eq 'installed binaries' }).hint -match 'quarantine'))
    $d3 = & $ctl doctor --fix --json | ConvertFrom-Json
    Check 'doctor --fix restores it from the build folder' ((Test-Path (Join-Path $binDir 'discordctl-shim.exe')) -and ($d3.fixed -match 'discordctl-shim.exe'))
    & $ctl doctor --json | Out-Null
    Check 'doctor is clean again' ($LASTEXITCODE -eq 0)

    Write-Host "== Discord starts and rewrites the link handler"
    & "$root\Update.exe" --processStart Discord.exe
    Check 'Discord started with CDP pipe' (Wait-Until { (Count 'discord CDP') -eq 1 })
    $repairLog = Join-Path $env:DISCORDCTL_HOME 'daemon.log'
    Start-Sleep -Milliseconds 2500
    Set-ItemProperty -Path $keyPs -Name '(default)' -Value $versioned     # то, что делает Discord при запуске
    Check 'daemon restores the handler by itself' (Wait-Until { (Handler) -eq $want } 8)

    Write-Host "== a discord:// link opened through the handler reaches the running instance"
    $plain = Count 'PLAIN'
    $cmd = (Handler).Replace('%1', 'discord://-/channels/9/9')
    cmd.exe /c "`"$cmd`"" 2>&1 | Out-Null
    Check 'link forwarded to the managed instance' (Wait-Until { (Count 'PLAIN') -gt $plain -and (Log) -match 'discord://-/channels/9/9' })
    Start-Sleep -Milliseconds 2500

    Write-Host "== uninstall while Discord runs"
    $u = & $ctl uninstall --json | ConvertFrom-Json
    $s = Shim
    Check 'originals restored on both targets' ($s.update -eq 'not_installed' -and $s.stub -eq 'not_installed')
    Check 'uninstall does not close Discord or kill the daemon' (($u.daemon -match 'exits by itself') -and ((& $ctl status --json | ConvertFrom-Json).daemon_running))
    Check 'handler is left on the stable root stub (now the original one)' ((Handler) -eq $want)
    Check 'doctor now says the shim is not installed' ((& $ctl doctor --json | ConvertFrom-Json).checks | Where-Object { $_.name -eq 'Update.exe shim' -and $_.level -eq 'info' })
    KillManaged
    Check 'daemon exits on its own after Discord closes' (Wait-Until { -not (& $ctl status --json | ConvertFrom-Json).daemon_running } 8)
    Check 'uninstall twice is harmless' ((& $ctl uninstall --json | ConvertFrom-Json).changed -eq $false)
    $purge = & $ctl uninstall --purge 2>&1 | Out-String
    Check '--purge refuses to run without an interactive terminal' ($LASTEXITCODE -ne 0 -and $purge -match 'interactive terminal')
    Check '--purge changed nothing' (Test-Path (Join-Path $env:DISCORDCTL_HOME 'config.json'))
}
finally {
    & $ctl daemon stop --force 2>$null | Out-Null
    Get-Process -Name 'Discord', 'discordctl-daemon' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$tmp*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    Remove-Item "HKCU:\Software\discordctl-test" -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "== daemon.log (tail)"
    $f = Join-Path $env:DISCORDCTL_HOME 'daemon.log'
    if (Test-Path $f) { Get-Content $f | Select-Object -Last 12 }
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
$expected = (Select-String -Path $PSCommandPath -Pattern "^\s*Check '").Count
if ($script:total -lt $expected) { Write-Host "only $script:total of $expected checks ran" -ForegroundColor Red; $script:fail++ }
if ($script:fail -gt 0) { Write-Host "$script:fail check(s) FAILED" -ForegroundColor Red; exit 1 }
Write-Host "ALL $script:total CHECKS PASSED" -ForegroundColor Green
