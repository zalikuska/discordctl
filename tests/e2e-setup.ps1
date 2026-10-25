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
