<#
.SYNOPSIS
  One command for a new PC: fetch everything Floaty needs, then run setup-floaty.ps1.

.DESCRIPTION
  Clones (or fast-forwards) the Floaty fork, mybrain (private settings) and jx
  (YouTube Music + the WhatsApp permission), installs the jx client, then runs
  tools\setup-floaty.ps1 from the fresh checkout. Safe to run again: a clean
  checkout is fast-forwarded, one with local work is only fetched.

  Works straight from the web:
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/glatinone/oyenza-studio/perf/lighter-glass-word-keys/tools/bootstrap-floaty.ps1)))

  The switches (-SkipBuild, -NoStart, ...) are passed on to setup-floaty.ps1.
#>
param(
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
function Step($m) { Write-Host "-> $m" -ForegroundColor Cyan }
function Note($m) { Write-Host "   $m" -ForegroundColor DarkGray }

# git (Git for Windows also brings Git Bash for the jx client and the credential manager).
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Step 'installing git'
    winget install --id Git.Git -e --silent --accept-source-agreements --accept-package-agreements | Out-Null
    $env:PATH = "$env:ProgramFiles\Git\cmd;$env:ProgramFiles\Git\bin;$env:PATH"
}
# A logged-in gh lets git read the private repos without a prompt.
if (Get-Command gh -ErrorAction SilentlyContinue) { gh auth setup-git 2>$null | Out-Null }

function Sync($url, $dir, $branch) {
    if (Test-Path (Join-Path $dir '.git')) {
        Step "update $dir"
        $remote = (git -C $dir remote -v | Select-String -SimpleMatch $url | Select-Object -First 1)
        $name = if ($remote) { ($remote -split '\s+')[0] } else { git -C $dir remote add bootstrap $url; 'bootstrap' }
        git -C $dir fetch --quiet $name
        $dirty = git -C $dir status --porcelain --untracked-files=no
        if ($dirty) { Note 'local changes here: fetched only'; return }
        if ($branch -and (git -C $dir rev-parse --abbrev-ref HEAD) -ne $branch) {
            git -C $dir checkout --quiet $branch 2>$null
            if ($LASTEXITCODE -ne 0) { git -C $dir checkout --quiet -b $branch "$name/$branch" }
        }
        $cur = git -C $dir rev-parse --abbrev-ref HEAD
        git -C $dir merge --ff-only --quiet "$name/$cur" 2>$null
        if ($LASTEXITCODE -ne 0) { Note "not a fast-forward of $name/${cur}: left as is" }
    } else {
        Step "clone $url"
        New-Item -ItemType Directory -Force (Split-Path $dir) | Out-Null
        $b = if ($branch) { @('--branch', $branch) } else { @() }
        git clone --quiet @b $url $dir
        if ($LASTEXITCODE -ne 0) { throw "clone failed: $url" }
    }
}

$floaty = Join-Path $Tools 'oyenza-studio'
$jx = Join-Path $Tools 'jx'
Sync 'https://github.com/glatinone/oyenza-studio.git' $floaty 'perf/lighter-glass-word-keys'
Sync 'https://github.com/glatinone/mybrain.git' $Brain $null
Sync 'https://github.com/glatinone/jx.git' $jx $null

# jx client (~/.local/bin/jx); its daemon starts itself on first use.
$bash = @("$env:ProgramFiles\Git\bin\bash.exe", (Get-Command bash -ErrorAction SilentlyContinue).Source) |
    Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if ($bash -and (Test-Path (Join-Path $jx 'install.sh'))) {
    Step 'jx client'
    & $bash -lc "bash '$(($jx -replace '\\', '/'))/install.sh'" | Out-Null
}

Step 'setup-floaty'
$pass = @{}
foreach ($k in 'SkipBuild', 'ForceSettings', 'NoAutostart', 'NoStart', 'NoWhatsApp') {
    if ($PSBoundParameters.ContainsKey($k)) { $pass[$k] = $PSBoundParameters[$k] }
}
& (Join-Path $floaty 'tools\setup-floaty.ps1') -Settings (Join-Path $Brain 'SETUP\floaty\settings.json') -Jx $jx @pass
