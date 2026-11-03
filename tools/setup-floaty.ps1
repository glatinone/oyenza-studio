<#
.SYNOPSIS
  Set Floaty up on a Windows PC from this checkout: toolchain, build, install,
  launcher, settings and start at sign-in. Safe to run again (it updates in place).

.DESCRIPTION
  No admin needed. Builds with the GNU toolchain (no Visual Studio), installs to
  %LOCALAPPDATA%\Programs\OyenzaStudio\poni.exe (the folder keeps its old name so
  older launchers keep working), writes start.vbs next to it, restores
  settings.json when one is given and none is there yet, and starts Floaty about
  90 s after sign-in.

.PARAMETER Settings
  A settings.json to restore into %APPDATA%\Poni (only when that has none, unless
  -ForceSettings). Defaults to ~\mybrain\SETUP\floaty\settings.json when present.

.PARAMETER ClaudeConfig
  The Claude Code config folder the AI tab reads (CLAUDE_CONFIG_DIR). Defaults to
  ~\.claude-loka when it exists, else Claude Code's own default.

.EXAMPLE
  pwsh -File tools\setup-floaty.ps1
  pwsh -File tools\setup-floaty.ps1 -SkipBuild      # reinstall the last build
#>
param(
    [string]$Settings = (Join-Path $HOME 'mybrain\SETUP\floaty\settings.json'),
    [string]$ClaudeConfig = (Join-Path $HOME '.claude-loka'),
    [switch]$SkipBuild,
    [switch]$ForceSettings,
    [switch]$NoAutostart,
    [switch]$NoStart
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$target = 'x86_64-pc-windows-gnu'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\OyenzaStudio'
$exe = Join-Path $installDir 'poni.exe'
function Step($m) { Write-Host "-> $m" -ForegroundColor Cyan }
function Note($m) { Write-Host "   $m" -ForegroundColor DarkGray }

# 1. Toolchain: Rust with the GNU target, and a MinGW gcc (WinLibs) on PATH.
if (-not $SkipBuild) {
    Step 'toolchain'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Note 'installing Rust (rustup, per user)'
        winget install --id Rustlang.Rustup -e --silent --accept-source-agreements --accept-package-agreements | Out-Null
        $env:PATH = "$HOME\.cargo\bin;$env:PATH"
    }
    if (-not ((rustup target list --installed) -contains $target)) { rustup target add $target | Out-Null }
    if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
        Note 'installing WinLibs (MinGW gcc, per user)'
        winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e --silent --accept-source-agreements --accept-package-agreements | Out-Null
        $gcc = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter gcc.exe -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($gcc) { $env:PATH = "$($gcc.DirectoryName);$env:PATH" }
    }
    if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) { throw 'gcc not found: install WinLibs (winget BrechtSanders.WinLibs.POSIX.UCRT) and open a new shell' }

    # 2. Build: thin LTO and 16 codegen units take minutes, not the release
    # profile's half hour, at below-normal priority so the PC stays usable.
    Step 'build (a few minutes)'
    $env:CARGO_PROFILE_RELEASE_LTO = 'thin'
    $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '16'
    Push-Location $repo
    try {
        $p = Start-Process cargo -ArgumentList 'build', '-p', 'poni', '--release', '--locked', '--target', $target -NoNewWindow -PassThru
        try { $p.PriorityClass = 'BelowNormal' } catch {}
        $p.WaitForExit()
        if ($p.ExitCode -ne 0) { throw "cargo build failed ($($p.ExitCode))" }
    } finally { Pop-Location }
}
$built = Join-Path $repo "target\$target\release\poni.exe"
if (-not (Test-Path $built)) { throw "no build at $built (run without -SkipBuild)" }

# 3. Install: stop the running copy, keep the old exe as poni.prev.exe.
Step "install to $installDir"
New-Item -ItemType Directory -Force $installDir | Out-Null
$running = Get-Process poni -ErrorAction SilentlyContinue
if ($running) {
    # A plain close first: Floaty saves its settings on the way out.
    & taskkill /IM poni.exe 2>$null | Out-Null
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    if (Get-Process poni -ErrorAction SilentlyContinue) { & taskkill /F /IM poni.exe 2>$null | Out-Null; Start-Sleep 1 }
}
if (Test-Path $exe) { Copy-Item $exe (Join-Path $installDir 'poni.prev.exe') -Force }
Copy-Item $built $exe -Force

# 4. Launcher: start.vbs sets CLAUDE_CONFIG_DIR for the AI tab, then starts Floaty.
Step 'launcher (start.vbs)'
$envLine = if (Test-Path $ClaudeConfig) { "sh.Environment(""Process"")(""CLAUDE_CONFIG_DIR"") = ""$ClaudeConfig""" } else { "' Claude Code's default config (no CLAUDE_CONFIG_DIR)." }
@"
' Floaty (dynamic island), installed by tools\setup-floaty.ps1.
Set sh = CreateObject("WScript.Shell")
$envLine
sh.Run """" & sh.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\Programs\OyenzaStudio\poni.exe""", 1, False
"@ | Set-Content -Encoding ASCII (Join-Path $installDir 'start.vbs')

# 5. Settings: restored only onto a PC without its own (Floaty rewrites the file on exit).
$dataDir = Join-Path $env:APPDATA 'Poni'
$dest = Join-Path $dataDir 'settings.json'
if ($Settings -and (Test-Path $Settings) -and ($ForceSettings -or -not (Test-Path $dest))) {
    Step "settings from $Settings"
    New-Item -ItemType Directory -Force $dataDir | Out-Null
    if (Test-Path $dest) { Copy-Item $dest "$dest.bak-setup" -Force }
    Copy-Item $Settings $dest -Force
} else { Note 'settings: keeping the ones here' }

# 6. Start at sign-in, ~90 s late so sign-in itself stays quick. A delayed-startup.vbs
# queue (Kiell's laptops) gets one line; otherwise a small Startup script of its own.
if (-not $NoAutostart) {
    Step 'start at sign-in'
    $startup = [Environment]::GetFolderPath('Startup')
    $queue = Join-Path $startup 'delayed-startup.vbs'
    $line = 'sh.Run "wscript.exe """ & L & "\Programs\OyenzaStudio\start.vbs""", 0, False'
    if ((Test-Path $queue) -and (Select-String -Path $queue -SimpleMatch 'OyenzaStudio\start.vbs' -Quiet)) {
        Note 'already in delayed-startup.vbs'
    } else {
        @'
' Floaty at sign-in, ~90 s late (tools\setup-floaty.ps1).
Set sh = CreateObject("WScript.Shell")
WScript.Sleep 90000
sh.Run "wscript.exe """ & sh.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\Programs\OyenzaStudio\start.vbs""", 0, False
'@ | Set-Content -Encoding ASCII (Join-Path $startup 'floaty-startup.vbs')
        Note 'added floaty-startup.vbs to the Startup folder'
    }
    Note "Floaty's own Start with Windows stays off (it would start a second copy without start.vbs)."
}

# 7. Optional helpers.
if (-not (Test-Path (Join-Path $env:LOCALAPPDATA 'jx\token'))) {
    Note 'jx is not running here: music search will open songs as links instead of in the YouTube Music tab'
}
Note 'WhatsApp in Edge: allow web.whatsapp.com notifications once (Edge > Site permissions) for chats on the island'

if (-not $NoStart) {
    Step 'start'
    Start-Process wscript.exe -ArgumentList "`"$(Join-Path $installDir 'start.vbs')`""
}
Write-Host 'Floaty is set up.' -ForegroundColor Green
