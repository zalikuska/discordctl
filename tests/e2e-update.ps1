# E2E-проверка `discordctl update` на ПОДДЕЛЬНОЙ установке Discord во временной папке. Нужен интернет: релизы
# v0.1.1 и v0.1.2 скачиваются с GitHub по-настоящему (проверка SHA-256, распаковка, замена занятого exe).
# Настоящий Discord, настоящий реестр и настоящая папка discordctl не затрагиваются: DISCORDCTL_*.
# Запуск: cargo build --release; powershell -File tests\e2e-update.ps1

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$src = Join-Path $repo 'target\release'
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$tmp = Join-Path $env:TEMP "dctl-upd-$id"
$root = Join-Path $tmp 'Discord'
$env:DISCORDCTL_HOME = Join-Path $tmp 'home'
$env:DISCORDCTL_DISCORD_ROOT = $root
$env:DISCORDCTL_PIPE = "\\.\pipe\discordctl-upd-$id"
$script:fail = 0
$script:total = 0

$run = Join-Path $tmp 'bin'
New-Item -ItemType Directory -Force $run | Out-Null
foreach ($n in 'discordctl', 'discordctl-shim', 'discordctl-daemon', 'discordctl-mcp', 'fake-discord') { Copy-Item (Join-Path $src "$n.exe") $run }
$ctl = Join-Path $run 'discordctl.exe'
$installed = Join-Path $env:DISCORDCTL_HOME 'bin\discordctl.exe'

function Check($name, $cond) {
    $script:total++
    if ($cond) { Write-Host "  PASS  $name" -ForegroundColor Green }
    else { Write-Host "  FAIL  $name" -ForegroundColor Red; $script:fail++ }
}
function Version($exe) { ((& $exe --version) -replace '^\S+\s+', '').Trim() }
$mine = Version $ctl

try {
    Write-Host "== setup: fake Discord in $root"
    New-Item -ItemType Directory -Force "$root\app-1.0.1" | Out-Null
    $fake = Join-Path $run 'fake-discord.exe'
    Copy-Item $fake "$root\Update.exe"
    Copy-Item $fake "$root\Discord.exe"
    Copy-Item $fake "$root\app-1.0.1\Discord.exe"

    Write-Host "== nothing to update on a PC without discordctl"
    $r = & $ctl update --release v0.1.1 --yes 2>&1 | Out-String
    Check 'not installed: it only unpacks and says how to install' (($LASTEXITCODE -eq 0) -and ($r -match 'not installed') -and (Test-Path (Join-Path $env:DISCORDCTL_HOME 'updates\v0.1.1\discordctl.exe')))

    Write-Host "== check"
    $c = & $ctl update --check --json | ConvertFrom-Json
    Check '--check reports versions and does not install' ($c.current -eq $mine -and $c.latest -match '^v\d' -and ($c.update_available -is [bool]))

    Write-Host "== bad input is refused before any download"
    & $ctl update --release 'v1/../x' --yes 2>&1 | Out-Null
    Check 'a tag with path characters is rejected' ($LASTEXITCODE -ne 0)
    & $ctl update --release v9.9.9 --yes 2>&1 | Out-Null
    Check 'a release that does not exist fails cleanly' ($LASTEXITCODE -ne 0)

    Write-Host "== install this build, then replace it from a running copy in bin\"
    & $ctl install --root $root --json | Out-Null
    Check 'this build is installed' ((Test-Path $installed) -and (Version $installed) -eq $mine)
    $out = & $installed update --release v0.1.1 --yes 2>&1 | Out-String
    Check 'the running bin\discordctl.exe replaced itself with v0.1.1' (($LASTEXITCODE -eq 0) -and (Version $installed) -eq '0.1.1')
    $s = (& $installed status --json | ConvertFrom-Json).shim
    Check 'the launch shims are still installed' ($s.update -eq 'installed' -and $s.stub -eq 'installed')
    Check 'the downloaded files were cleaned up' (-not (Test-Path (Join-Path $env:DISCORDCTL_HOME 'updates\v0.1.1')))

    Write-Host "== update again from the build outside bin\ (the old file left behind is cleaned up)"
    & $ctl update --release v0.1.2 --yes 2>&1 | Out-Null
    Check 'now v0.1.2 is installed' (($LASTEXITCODE -eq 0) -and (Version $installed) -eq '0.1.2')
    Check 'the daemon binary is the new one too' ((& $ctl --json status | ConvertFrom-Json).shim.update -eq 'installed')
    & $ctl install --root $root --json | Out-Null
    $left = @(Get-ChildItem (Join-Path $env:DISCORDCTL_HOME 'bin') -Filter '*.old')
    Check 'no .old leftovers after the next install' ($left.Count -eq 0)
}
finally {
    & $ctl daemon stop --force 2>$null | Out-Null
    Start-Sleep -Milliseconds 300
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
$expected = (Select-String -Path $PSCommandPath -Pattern "^\s*Check '").Count
if ($script:total -lt $expected) { Write-Host "only $script:total of $expected checks ran" -ForegroundColor Red; $script:fail++ }
if ($script:fail -gt 0) { Write-Host "$script:fail check(s) FAILED" -ForegroundColor Red; exit 1 }
Write-Host "ALL $script:total CHECKS PASSED" -ForegroundColor Green
