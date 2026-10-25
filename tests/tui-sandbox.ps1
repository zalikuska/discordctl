# Opens the full-screen setup wizard against a FAKE Discord in a temporary folder, so you can click through it
# without touching your real Discord or your real discordctl data (DISCORDCTL_* variables point everything at the sandbox).
# Run: cargo build --release; powershell -File tests\tui-sandbox.ps1
param([switch]$Keep, [string]$Keys = '')
$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$src = Join-Path $repo 'target\release'
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$tmp = Join-Path $env:TEMP "dctl-tui-$id"
$root = Join-Path $tmp 'Discord'
$run = Join-Path $tmp 'bin'
New-Item -ItemType Directory -Force "$root\app-1.0.1", $run | Out-Null
foreach ($n in 'discordctl', 'discordctl-shim', 'discordctl-daemon', 'discordctl-mcp', 'fake-discord') { Copy-Item (Join-Path $src "$n.exe") $run }
foreach ($t in "$root\Update.exe", "$root\Discord.exe", "$root\app-1.0.1\Discord.exe") { Copy-Item (Join-Path $run 'fake-discord.exe') $t }

$env:DISCORDCTL_HOME = Join-Path $tmp 'home'
$env:DISCORDCTL_DISCORD_ROOT = $root
$env:DISCORDCTL_PIPE = "\\.\pipe\discordctl-tui-$id"
$env:DISCORDCTL_HANDLER_KEY = "Software\discordctl-test\tui-$id"
$env:DISCORDCTL_SETUP_WAIT = '6'
# -Keys 'enter down enter ...' plays the key presses by itself (used for automatic checks)
if ($Keys) { $env:DISCORDCTL_TUI_KEYS = $Keys }

try {
    & (Join-Path $run 'discordctl.exe') setup
}
finally {
    & (Join-Path $run 'discordctl.exe') daemon stop --force 2>$null | Out-Null
    Get-Process -Name 'Discord', 'discordctl-daemon' -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -like "$tmp*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item "HKCU:\Software\discordctl-test\tui-$id" -Recurse -Force -ErrorAction SilentlyContinue
    if (-not $Keep) { Start-Sleep -Milliseconds 500; Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }
}
