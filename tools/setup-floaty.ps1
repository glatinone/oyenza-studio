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

.PARAMETER Jx
  A jx checkout (Kiell's private browser engine). Used for YouTube Music and, when
  Edge is open, to answer the WhatsApp notification prompt. tools\bootstrap-floaty.ps1
  fetches it.

.PARAMETER ClaudeConfig
  The Claude Code config folder the AI tab reads (CLAUDE_CONFIG_DIR). Defaults to
  ~\.claude-loka when it exists, else Claude Code's own default.

.EXAMPLE
  pwsh -File tools\setup-floaty.ps1
  pwsh -File tools\setup-floaty.ps1 -SkipBuild      # reinstall the last build
  irm https://raw.githubusercontent.com/glatinone/oyenza-studio/HEAD/install.ps1 | iex   # any PC
#>
param(
    [string]$Settings = (Join-Path $HOME 'mybrain\SETUP\floaty\settings.json'),
    [string]$ClaudeConfig = (Join-Path $HOME '.claude-loka'),
    [string]$Jx = (Join-Path $HOME 'Videos\Tools\jx'),
    [switch]$SkipBuild,
    [switch]$ForceSettings,
    [switch]$NoAutostart,
    [switch]$NoWhatsApp,
    [switch]$NoStart
)
$ErrorActionPreference = 'Stop'
trap { Write-Host "Floaty setup stopped: $($_.Exception.Message)" -ForegroundColor Red; exit 1 }
$ProgressPreference = 'SilentlyContinue'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072 } catch {}
$repo = Split-Path -Parent $PSScriptRoot
$target = 'x86_64-pc-windows-gnu'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\OyenzaStudio'
$exe = Join-Path $installDir 'poni.exe'
function Step($m) { Write-Host "-> $m" -ForegroundColor Cyan }
function Note($m) { Write-Host "   $m" -ForegroundColor DarkGray }
# A native tool with its output swallowed; stderr too, without Windows PowerShell
# 5.1 turning a stderr line into an error under 'Stop'. Returns the exit code.
function Quiet($exe) {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { & $exe @args 2>&1 | Out-Null } finally { $ErrorActionPreference = $eap }
    $LASTEXITCODE
}

# 1. Toolchain: Rust's GNU toolchain (no Visual Studio needed) and a MinGW gcc
# (WinLibs), both per user.
$toolchain = 'stable-x86_64-pc-windows-gnu'
if (-not $SkipBuild) {
    Step 'toolchain'
    $env:PATH = "$HOME\.cargo\bin;$env:PATH"
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        Note 'installing Rust (rustup, per user)'
        $init = Join-Path $env:TEMP 'rustup-init.exe'
        Invoke-WebRequest -UseBasicParsing 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe' -OutFile $init
        $rc = Quiet $init -y --default-host x86_64-pc-windows-gnu --default-toolchain stable --profile minimal
        Remove-Item $init -ErrorAction SilentlyContinue
        if ($rc -ne 0) { throw "Rust did not install (rustup-init exit $rc)" }
    }
    # A PC with the MSVC Rust already: add the GNU one next to it and build with that.
    if (-not ((rustup toolchain list) -match "^$toolchain")) {
        Note 'adding the Rust GNU toolchain'
        if ((Quiet rustup toolchain install $toolchain --profile minimal) -ne 0) { throw 'the Rust GNU toolchain did not install' }
    }
    $env:RUSTUP_TOOLCHAIN = $toolchain

    $mingw = 'x86_64-w64-mingw32-gcc'
    function Find-Gcc {
        if (Get-Command $mingw -ErrorAction SilentlyContinue) { return $true }
        $hit = @("$env:LOCALAPPDATA\Microsoft\WinGet\Packages", "$env:LOCALAPPDATA\Programs\WinLibs") | Where-Object { Test-Path $_ } |
            ForEach-Object { Get-ChildItem $_ -Recurse -Filter "$mingw.exe" -ErrorAction SilentlyContinue } | Select-Object -First 1
        if ($hit) { $env:PATH = "$($hit.DirectoryName);$env:PATH"; return $true }
        $false
    }
    if (-not (Find-Gcc) -and (Get-Command winget -ErrorAction SilentlyContinue)) {
        Note 'installing WinLibs (MinGW gcc, per user)'
        winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e --silent --accept-source-agreements --accept-package-agreements | Out-Null
    }
    if (-not (Find-Gcc)) {
        # No winget: the same WinLibs build straight from its GitHub releases.
        Note 'downloading WinLibs (MinGW gcc, per user)'
        $rel = Invoke-RestMethod -UseBasicParsing 'https://api.github.com/repos/brechtsanders/winlibs_mingw/releases/latest'
        $asset = $rel.assets | Where-Object { $_.name -match '^winlibs-x86_64-posix-seh-.*ucrt.*\.zip$' -and $_.name -notmatch 'llvm' } | Select-Object -First 1
        if (-not $asset) { throw 'no WinLibs download found' }
        $zip = Join-Path $env:TEMP $asset.name
        Invoke-WebRequest -UseBasicParsing $asset.browser_download_url -OutFile $zip
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $dest = Join-Path $env:LOCALAPPDATA 'Programs\WinLibs'
        if (Test-Path $dest) { Remove-Item $dest -Recurse -Force }
        [IO.Compression.ZipFile]::ExtractToDirectory($zip, $dest)
        Remove-Item $zip -ErrorAction SilentlyContinue
    }
    if (-not (Find-Gcc)) { throw 'the MinGW gcc (WinLibs) could not be set up' }

    # 2. Build: thin LTO and 16 codegen units take minutes, not the release
    # profile's half hour, at below-normal priority so the PC stays usable.
    Step 'building Floaty (10-15 min the first time, a few after that; the PC stays usable)'
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
if (-not (Test-Path $built)) { throw "no build yet at $built (skip-build mode)" }

# 3. Install: stop the running copy, keep the old exe as poni.prev.exe.
Step "install to $installDir"
New-Item -ItemType Directory -Force $installDir | Out-Null
$running = Get-Process poni -ErrorAction SilentlyContinue
if ($running) {
    # A plain close first: Floaty saves its settings on the way out.
    $null = Quiet taskkill /IM poni.exe
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    if (Get-Process poni -ErrorAction SilentlyContinue) { $null = Quiet taskkill /F /IM poni.exe; Start-Sleep 1 }
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
if ((Test-Path $Jx) -and -not (Test-Path (Join-Path $env:LOCALAPPDATA 'jx\token'))) {
    Note 'jx is not running here: music search will open songs as links instead of in the YouTube Music tab'
}
# WhatsApp Web toasts reach the island only when Edge allows the site's notifications.
if (-not $NoWhatsApp) {
    Step 'WhatsApp notifications in Edge'
    # The Microsoft Store's python.exe placeholder doesn't count.
    $py = Get-Command python, py -CommandType Application -ErrorAction SilentlyContinue |
        Where-Object { $_.Source -notlike '*\WindowsApps\*' } | Select-Object -First 1
    if ($py) {
        $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
        try {
            & $py.Source (Join-Path $PSScriptRoot 'edge-allow-notifications.py') 'https://web.whatsapp.com' --jx $Jx 2>&1 | ForEach-Object { Note "$_" }
        } catch { Note 'skipped this time' } finally { $ErrorActionPreference = $eap }
    } else { Note 'skipped (needs Python)' }
}

if (-not $NoStart) {
    Step 'start'
    Start-Process wscript.exe -ArgumentList "`"$(Join-Path $installDir 'start.vbs')`""
    Write-Host "Floaty is running $([char]0x2014) point at the top of your screen." -ForegroundColor Green
} elseif (-not $NoAutostart) {
    Write-Host 'Floaty is installed; it starts at your next sign-in.' -ForegroundColor Green
} else {
    Write-Host 'Floaty is installed.' -ForegroundColor Green
}
