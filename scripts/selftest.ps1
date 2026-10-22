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
    Write-Host "GROQ_API_KEY is not set. Live Whisper and Llama tests will SKIP."
} else {
    Write-Host "GROQ_API_KEY is set. Live Whisper and Llama tests will run."
}

# The first group includes a cold compile. Later limits assume the crate is built.
$steps = @(
    @{ Name = "Microphone selection and capture"; Filter = "audio::"; TimeoutSec = 900 },
    @{ Name = "Credential store round trip"; Filter = "credentials::"; TimeoutSec = 180 },
    @{ Name = "Paste into a text control"; Filter = "insert::"; TimeoutSec = 90 },
    @{ Name = "Groq transcription, cleanup, and pipeline"; Filter = "live_tests::"; TimeoutSec = 300 },
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

Write-Host ""
Write-Host "========"
foreach ($name in $passed) { Write-Host "PASS  $name" }
foreach ($name in $failed) { Write-Host "FAIL  $name" }
Write-Host ("{0} passed, {1} failed" -f $passed.Count, $failed.Count)
if ($failed.Count -gt 0) { exit 1 }
exit 0
