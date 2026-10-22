# Run Cadence's Windows checks and print a pass/fail report.
# Live Groq tests run only when GROQ_API_KEY is set in the environment.
# Each group has its own time limit. A hang is killed so the later groups still run.
# Usage, from the repo root or from anywhere:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/selftest.ps1

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Cadence self-test"
Write-Host "Repo: $root"
if ([string]::IsNullOrWhiteSpace($env:GROQ_API_KEY)) {
    Write-Host "GROQ_API_KEY is not set. Live Whisper, Llama, and latency tests will SKIP. Encode timing still runs."
} else {
    Write-Host "GROQ_API_KEY is set. Live Whisper, Llama, and latency tests will run."
}

# The first group includes a cold compile. Later limits assume the crate is built.
$steps = @(
    @{ Name = "Microphone selection and capture"; Filter = "audio::"; TimeoutSec = 900 },
    @{ Name = "Credential store round trip"; Filter = "credentials::"; TimeoutSec = 180 },
    @{ Name = "Paste into a text control"; Filter = "insert::"; TimeoutSec = 90 },
    @{ Name = "Groq transcription, cleanup, pipeline, and latency"; Filter = "live_tests::"; TimeoutSec = 300 },
    @{ Name = "Other unit tests"; Filter = ""; TimeoutSec = 300 }
)

function Invoke-LimitedCargo {
    param(
        [string]$Name,
        [string]$Filter,
        [int]$TimeoutSec
    )

    $cargoArgs = @("test", "--manifest-path", "src-tauri/Cargo.toml", "--lib")
    if ($Filter -ne "") {
        $cargoArgs += $Filter
    }
    $cargoArgs += @("--", "--test-threads=1", "--nocapture")
    if ($Filter -eq "") {
        $cargoArgs += @(
            "--skip", "audio::",
            "--skip", "credentials::",
            "--skip", "insert::",
            "--skip", "live_tests::"
        )
    }

    Write-Host ""
    Write-Host "---- $Name (limit ${TimeoutSec}s) ----"

    $cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if (-not $cargo) {
        $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    }
    if (-not $cargo) {
        Write-Host "FAIL  $Name"
        Write-Host "cargo was not found on PATH."
        return $false
    }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    # Full path: Windows PowerShell 5.1 does not search PATH when UseShellExecute is false.
    $psi.FileName = $cargo.Source
    $psi.Arguments = ($cargoArgs -join " ")
    $psi.WorkingDirectory = $root
    $psi.UseShellExecute = $false

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    try {
        [void]$proc.Start()
    } catch {
        Write-Host "FAIL  $Name"
        Write-Host "Could not start cargo: $($_.Exception.Message)"
        return $false
    }

    $limitMs = [Math]::Min($TimeoutSec * 1000, [int]::MaxValue)
    if (-not $proc.WaitForExit($limitMs)) {
        Write-Host "FAIL  $Name"
        Write-Host "Timed out after ${TimeoutSec}s. Stopping this group so the rest of the suite can run."
        & taskkill.exe /F /T /PID $proc.Id 2>&1 | ForEach-Object { Write-Host $_ }
        try { $proc.WaitForExit(10000) | Out-Null } catch {}
        return $false
    }
    if ($proc.ExitCode -eq 0) {
        Write-Host "PASS  $Name"
        return $true
    }
    Write-Host "FAIL  $Name (exit $($proc.ExitCode))"
    return $false
}

function Invoke-StartupSmoke {
    $name = "Startup smoke: release app get_settings"
    $limitSec = 1500
    Write-Host ""
    Write-Host "---- $name (limit ${limitSec}s) ----"

    $npm = Get-Command npm.cmd -ErrorAction SilentlyContinue
    if (-not $npm) { $npm = Get-Command npm -ErrorAction SilentlyContinue }
    if (-not $npm) {
        Write-Host "FAIL  $name"
        Write-Host "npm was not found on PATH."
        return $false
    }
    $cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if (-not $cargo) { $cargo = Get-Command cargo -ErrorAction SilentlyContinue }
    if (-not $cargo) {
        Write-Host "FAIL  $name"
        Write-Host "cargo was not found on PATH."
        return $false
    }

    $started = Get-Date
    & $npm.Source run build
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL  $name"
        Write-Host "npm run build failed."
        return $false
    }
    & $cargo.Source build --release --manifest-path src-tauri/Cargo.toml
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL  $name"
        Write-Host "cargo build --release failed."
        return $false
    }

    $exe = Join-Path $root "src-tauri\target\release\cadence.exe"
    if (-not (Test-Path $exe)) {
        Write-Host "FAIL  $name"
        Write-Host "Missing $exe"
        return $false
    }

    $work = Join-Path ([System.IO.Path]::GetTempPath()) ("cadence-smoke-" + [guid]::NewGuid().ToString("n"))
    New-Item -ItemType Directory -Path $work | Out-Null
    $result = Join-Path $work "result.txt"
    $pretty = @'
{
    "hotkey": {
        "ctrl": true,
        "alt": false,
        "shift": false,
        "win": true,
        "key": null
    },
    "microphone": null,
    "model": "whisper-large-v3-turbo",
    "cleanupEnabled": true,
    "language": "auto",
    "dictionary": [],
    "launchAtStartup": false
}
'@
    $utf8Bom = New-Object System.Text.UTF8Encoding $true
    [System.IO.File]::WriteAllText((Join-Path $work "settings.json"), $pretty.Trim(), $utf8Bom)

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.WorkingDirectory = $root
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["CADENCE_SMOKE"] = "1"
    $psi.EnvironmentVariables["CADENCE_SMOKE_RESULT"] = $result
    $psi.EnvironmentVariables["CADENCE_CONFIG_DIR"] = $work
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    try {
        [void]$proc.Start()
    } catch {
        Write-Host "FAIL  $name"
        Write-Host "Could not start ${exe}: $($_.Exception.Message)"
        return $false
    }

    $elapsed = [int]((Get-Date) - $started).TotalSeconds
    $runLimit = [Math]::Max(90, $limitSec - $elapsed)
    if (-not $proc.WaitForExit($runLimit * 1000)) {
        Write-Host "FAIL  $name"
        Write-Host "The release app did not report get_settings within ${runLimit}s."
        & taskkill.exe /F /T /PID $proc.Id 2>&1 | ForEach-Object { Write-Host $_ }
        try { $proc.WaitForExit(10000) | Out-Null } catch {}
        return $false
    }

    if (-not (Test-Path $result)) {
        Write-Host "FAIL  $name"
        Write-Host "No smoke result (exit $($proc.ExitCode)). Cadence may already be running, or the settings page never invoked get_settings."
        return $false
    }
    $raw = [System.IO.File]::ReadAllText($result).Trim()
    if (-not $raw.StartsWith("OK ")) {
        Write-Host "FAIL  $name"
        Write-Host $raw
        return $false
    }
    try {
        $settings = $raw.Substring(3) | ConvertFrom-Json
    } catch {
        Write-Host "FAIL  $name"
        Write-Host "get_settings did not return JSON: $raw"
        return $false
    }
    if ($settings.cleanupEnabled -ne $true -or $settings.model -ne "whisper-large-v3-turbo") {
        Write-Host "FAIL  $name"
        Write-Host "Unexpected settings: $raw"
        return $false
    }
    if ($null -ne $settings.microphone) {
        Write-Host "FAIL  $name"
        Write-Host "microphone should be null: $raw"
        return $false
    }
    if (-not $settings.hotkey.ctrl -or -not $settings.hotkey.win) {
        Write-Host "FAIL  $name"
        Write-Host "hotkey was not Ctrl+Win: $raw"
        return $false
    }
    Write-Host "PASS  $name"
    Write-Host "get_settings $($raw.Substring(3))"
    return $true
}

$passed = @()
$failed = @()

foreach ($step in $steps) {
    $ok = Invoke-LimitedCargo -Name $step.Name -Filter $step.Filter -TimeoutSec $step.TimeoutSec
    if ($ok) {
        $passed += $step.Name
    } else {
        $failed += $step.Name
    }
}

$smokeName = "Startup smoke: release app get_settings"
$smokeOk = Invoke-StartupSmoke
if ($smokeOk) { $passed += $smokeName } else { $failed += $smokeName }

Write-Host ""
Write-Host "========"
foreach ($name in $passed) { Write-Host "PASS  $name" }
foreach ($name in $failed) { Write-Host "FAIL  $name" }
Write-Host ("{0} passed, {1} failed" -f $passed.Count, $failed.Count)
if ($failed.Count -gt 0) { exit 1 }
exit 0
