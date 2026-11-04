# Floaty for Windows, one line in PowerShell (no admin needed):
#
#   irm https://raw.githubusercontent.com/glatinone/oyenza-studio/HEAD/install.ps1 | iex
#
# Fetches tools/bootstrap-floaty.ps1 and runs it in a fresh PowerShell past the
# execution policy. That gets the source (git or a zip), the Rust GNU toolchain and
# MinGW gcc, builds Floaty, installs it to %LOCALAPPDATA%\Programs\OyenzaStudio,
# starts it and starts it at sign-in. Running it again updates Floaty.
#
# Options are environment variables, since | iex takes no arguments:
#   $env:FLOATY_DIR = 'D:\src\floaty'   where the source goes (default ~\floaty)
#   $env:FLOATY_SKIP_BUILD = 1          reinstall the last build only
#   $env:FLOATY_NO_START = 1            don't start Floaty now
#   $env:FLOATY_NO_AUTOSTART = 1        don't start it at sign-in
#   $env:FLOATY_NO_WHATSAPP = 1         leave Edge's notification settings alone
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072 } catch {}
    Write-Host 'Floaty: setting up (the first time takes 10-15 minutes)' -ForegroundColor Cyan

    $ref = if ($env:FLOATY_REF) { $env:FLOATY_REF } else { 'HEAD' }
    $script = Join-Path $env:TEMP "floaty-bootstrap-$PID.ps1"
    try {
        Invoke-WebRequest -UseBasicParsing "https://raw.githubusercontent.com/glatinone/oyenza-studio/$ref/tools/bootstrap-floaty.ps1" -OutFile $script
    } catch {
        Write-Host "Floaty: could not reach GitHub ($($_.Exception.Message))" -ForegroundColor Red
        return
    }
    $ps = (Get-Process -Id $PID).Path
    if (-not $ps -or (Split-Path -Leaf $ps) -notmatch '^(powershell|pwsh)\.exe$') {
        $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    }
    & $ps -NoProfile -ExecutionPolicy Bypass -File $script
    $code = $LASTEXITCODE
    Remove-Item $script -ErrorAction SilentlyContinue
    if ($code -ne 0) { Write-Host "Floaty: setup stopped (exit $code), details above" -ForegroundColor Red }
}
