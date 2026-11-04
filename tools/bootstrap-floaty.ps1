<#
.SYNOPSIS
  One command for any PC: fetch Floaty's source, then run setup-floaty.ps1.

.DESCRIPTION
  Gets git when it's missing (or downloads the source as a zip when git can't
  be installed), clones or fast-forwards the Floaty fork, then runs
  tools\setup-floaty.ps1 from that checkout. On a PC that can read Kiell's
  private repos (mybrain: settings, jx: YouTube Music and the WhatsApp
  permission) those come along too; access is checked without any login
  prompt, so everyone else simply skips them. Safe to run again: a clean
  checkout is fast-forwarded, one with local work is only fetched.

  Normally started by install.ps1 at the repo root:
    irm https://raw.githubusercontent.com/glatinone/oyenza-studio/HEAD/install.ps1 | iex

  Options also come from environment variables, so they work through | iex:
    FLOATY_DIR           the checkout folder
    FLOATY_SKIP_BUILD=1  reinstall the last build, no toolchain or cargo
    FLOATY_NO_START=1    don't start Floaty at the end
    FLOATY_NO_AUTOSTART=1, FLOATY_NO_WHATSAPP=1, FLOATY_FORCE_SETTINGS=1
    FLOATY_NO_PRIVATE=1  don't look for the private repos at all
#>
param(
    [string]$Dir = $env:FLOATY_DIR,
    [string]$Tools = (Join-Path $HOME 'Videos\Tools'),
    [string]$Brain = (Join-Path $HOME 'mybrain'),
    # Passed on to setup-floaty.ps1.
    [switch]$SkipBuild,
    [switch]$ForceSettings,
    [switch]$NoAutostart,
    [switch]$NoStart,
    [switch]$NoWhatsApp
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072 } catch {}
$branch = 'perf/lighter-glass-word-keys'
$forkUrl = 'https://github.com/glatinone/oyenza-studio.git'
function Step($m) { Write-Host "-> $m" -ForegroundColor Cyan }
function Note($m) { Write-Host "   $m" -ForegroundColor DarkGray }
function Flag($name) { $v = [Environment]::GetEnvironmentVariable($name); [bool]$v -and $v -notin '0', 'false', 'no' }

if (Flag 'FLOATY_SKIP_BUILD') { $SkipBuild = $true }
if (Flag 'FLOATY_NO_START') { $NoStart = $true }
if (Flag 'FLOATY_NO_AUTOSTART') { $NoAutostart = $true }
if (Flag 'FLOATY_NO_WHATSAPP') { $NoWhatsApp = $true }
if (Flag 'FLOATY_FORCE_SETTINGS') { $ForceSettings = $true }

# Checkout folder: FLOATY_DIR, else an existing checkout, else ~\Videos\Tools on a
# PC that keeps its tools there, else ~\floaty.
if (-not $Dir) {
    $old = Join-Path $Tools 'oyenza-studio'
    $Dir = if ((Test-Path (Join-Path $old '.git')) -or (Test-Path $Tools)) { $old } else { Join-Path $HOME 'floaty' }
}
$Dir = [IO.Path]::GetFullPath($Dir)

# git never asks for a login here: no terminal prompt, no credential window.
$env:GIT_TERMINAL_PROMPT = '0'
$env:GCM_INTERACTIVE = 'never'
Remove-Item Env:GIT_ASKPASS, Env:SSH_ASKPASS -ErrorAction SilentlyContinue

# Runs git with its stderr captured (Windows PowerShell 5.1 would turn a stderr
# line into an error under 'Stop'); the exit code stays in $LASTEXITCODE.
function G {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { & git -c credential.interactive=never @args 2>&1 | ForEach-Object { "$_" } } finally { $ErrorActionPreference = $eap }
}
function Refresh-Path {
    $m = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $u = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:PATH = "$m;$u;$env:LOCALAPPDATA\Programs\Git\cmd;$env:ProgramFiles\Git\cmd;$env:PATH"
}

# git: Git for Windows, per user when winget can, which also brings Git Bash for jx.
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Refresh-Path
    if (-not (Get-Command git -ErrorAction SilentlyContinue) -and (Get-Command winget -ErrorAction SilentlyContinue)) {
        Step 'installing Git'
        winget install --id Git.Git -e --silent --scope user --accept-source-agreements --accept-package-agreements | Out-Null
        if ($LASTEXITCODE -ne 0) { winget install --id Git.Git -e --silent --accept-source-agreements --accept-package-agreements | Out-Null }
        Refresh-Path
    }
}
$hasGit = [bool](Get-Command git -ErrorAction SilentlyContinue)
# A logged-in gh lets git read Kiell's private repos without a prompt.
if ($hasGit -and (Get-Command gh -ErrorAction SilentlyContinue)) {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { gh auth setup-git 2>&1 | Out-Null } finally { $ErrorActionPreference = $eap }
}

function Sync($url, $dir, $branch) {
    if (Test-Path (Join-Path $dir '.git')) {
        Step "updating $dir"
        $remote = G -C $dir remote -v | Where-Object { $_ -like "*$url*" } | Select-Object -First 1
        $name = if ($remote) { ($remote -split '\s+')[0] } else { $null = G -C $dir remote add bootstrap $url; 'bootstrap' }
        $null = G -C $dir fetch --quiet $name
        if ($LASTEXITCODE -ne 0) { Note 'offline? kept the copy here'; return }
        if (G -C $dir status --porcelain --untracked-files=no) { Note 'local changes here: fetched only'; return }
        if ($branch -and (G -C $dir rev-parse --abbrev-ref HEAD) -ne $branch) {
            $null = G -C $dir checkout --quiet $branch
            if ($LASTEXITCODE -ne 0) { $null = G -C $dir checkout --quiet -b $branch "$name/$branch" }
        }
        $cur = G -C $dir rev-parse --abbrev-ref HEAD
        $null = G -C $dir merge --ff-only --quiet "$name/$cur"
        if ($LASTEXITCODE -ne 0) { Note "not a fast-forward of $name/${cur}: left as is" }
    } else {
        Step "downloading $(($url -split '/')[-1] -replace '\.git$', '') to $dir"
        New-Item -ItemType Directory -Force (Split-Path $dir) | Out-Null
        $b = if ($branch) { @('--branch', $branch) } else { @() }
        $out = G clone --quiet @b $url $dir
        if ($LASTEXITCODE -ne 0) { $out | ForEach-Object { Note $_ }; throw "could not download $url" }
    }
}

# No git at all: the branch as a zip, unpacked over the folder (cargo's target\ stays).
function Sync-Zip($dir) {
    Step "downloading Floaty's source to $dir"
    $zip = Join-Path $env:TEMP "floaty-src-$PID.zip"
    $tmp = Join-Path $env:TEMP "floaty-src-$PID"
    Invoke-WebRequest -UseBasicParsing "https://codeload.github.com/glatinone/oyenza-studio/zip/refs/heads/$branch" -OutFile $zip
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::ExtractToDirectory($zip, $tmp)
    $top = Get-ChildItem $tmp -Directory | Select-Object -First 1
    New-Item -ItemType Directory -Force $dir | Out-Null
    & robocopy $top.FullName $dir /E /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "could not unpack the source into $dir" }
    Remove-Item $zip, $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$zipCopy = (Test-Path $Dir) -and -not (Test-Path (Join-Path $Dir '.git')) -and [bool](Get-ChildItem $Dir -Force | Select-Object -First 1)
if ($hasGit -and -not $zipCopy) { Sync $forkUrl $Dir $branch } else { Sync-Zip $Dir }

# Kiell's private repos: only when this PC can already read them (stored login).
$jx = Join-Path $Tools 'jx'
$jxSynced = $false
if ($hasGit -and -not (Flag 'FLOATY_NO_PRIVATE')) {
    $private = @(
        @{ Url = 'https://github.com/glatinone/mybrain.git'; Dir = $Brain },
        @{ Url = 'https://github.com/glatinone/jx.git'; Dir = $jx }
    )
    $readable = @($private | Where-Object { $null = G ls-remote --heads $_.Url; $LASTEXITCODE -eq 0 })
    if ($readable.Count -eq 0) { Note 'personal extras (settings sync, YouTube Music helper) are not part of this install' }
    foreach ($r in $readable) { Sync $r.Url $r.Dir $null; if ($r.Dir -eq $jx) { $jxSynced = $true } }
}

# jx client (~/.local/bin/jx); its daemon starts itself on first use.
$bash = @("$env:ProgramFiles\Git\bin\bash.exe", "$env:LOCALAPPDATA\Programs\Git\bin\bash.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if ($jxSynced -and $bash -and (Test-Path (Join-Path $jx 'install.sh'))) {
    Step 'jx client'
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { & $bash -lc "bash '$(($jx -replace '\\', '/'))/install.sh'" 2>&1 | Out-Null } finally { $ErrorActionPreference = $eap }
}

# setup-floaty.ps1 in a fresh PowerShell of the same kind, past the execution policy.
$ps = (Get-Process -Id $PID).Path
if ((Split-Path -Leaf $ps) -notmatch '^(powershell|pwsh)\.exe$') { $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe' }
$argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $Dir 'tools\setup-floaty.ps1'),
    '-Settings', (Join-Path $Brain 'SETUP\floaty\settings.json'), '-Jx', $jx)
if ($SkipBuild) { $argv += '-SkipBuild' }
if ($ForceSettings) { $argv += '-ForceSettings' }
if ($NoAutostart) { $argv += '-NoAutostart' }
if ($NoStart) { $argv += '-NoStart' }
if ($NoWhatsApp) { $argv += '-NoWhatsApp' }
& $ps @argv
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
