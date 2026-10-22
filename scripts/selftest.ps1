# Run Cadence's Windows checks and print a pass/fail report.
# Live Groq tests run only when GROQ_API_KEY is set in the environment.
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

$steps = @(
    @{ Name = "Microphone selection and capture"; Filter = "audio::" },
    @{ Name = "Credential store round trip"; Filter = "credentials::" },
    @{ Name = "Paste into a text control"; Filter = "insert::" },
    @{ Name = "Groq transcription, cleanup, and pipeline"; Filter = "live_tests::" },
    @{ Name = "Other unit tests"; Filter = "" }
)

$passed = @()
$failed = @()

foreach ($step in $steps) {
    Write-Host ""
    Write-Host "---- $($step.Name) ----"
    if ($step.Filter -eq "") {
        & cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1 --nocapture --skip audio:: --skip credentials:: --skip insert:: --skip live_tests::
    } else {
        & cargo test --manifest-path src-tauri/Cargo.toml $step.Filter -- --test-threads=1 --nocapture
    }
    if ($LASTEXITCODE -eq 0) {
        $passed += $step.Name
        Write-Host "PASS  $($step.Name)"
    } else {
        $failed += $step.Name
        Write-Host "FAIL  $($step.Name)"
    }
}

Write-Host ""
Write-Host "========"
foreach ($name in $passed) { Write-Host "PASS  $name" }
foreach ($name in $failed) { Write-Host "FAIL  $name" }
Write-Host ("{0} passed, {1} failed" -f $passed.Count, $failed.Count)
if ($failed.Count -gt 0) { exit 1 }
exit 0
