# Run Bolo.com's Windows checks and print a pass/fail report.
# Live Groq tests run only when GROQ_API_KEY is set in the environment.
# Each group has its own time limit. A hang is killed so the later groups still run.
# Usage, from the repo root or from anywhere:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/selftest.ps1

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Bolo.com self-test"
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
    @{ Name = "Settings and key carryover from Cadence"; Filter = "carryover::"; TimeoutSec = 180 },
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
            "--skip", "live_tests::",
            "--skip", "carryover::"
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

function Initialize-SmokeJob {
    if ("CadenceSmokeJob" -as [type]) { return $true }
    $source = @'
using System;
using System.Runtime.InteropServices;
public static class CadenceSmokeJob {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr lpJobAttributes, string lpName);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(IntPtr hJob, int JobObjectInfoClass, IntPtr lpJobObjectInfo, uint cbJobObjectInfoLength);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(IntPtr hJob, IntPtr hProcess);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr OpenProcess(uint dwDesiredAccess, bool bInheritHandle, int dwProcessId);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr hObject);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateJobObject(IntPtr hJob, uint uExitCode);
    const uint PROCESS_TERMINATE = 0x0001;
    const uint PROCESS_SET_QUOTA = 0x0100;
    const int JobObjectExtendedLimitInformation = 9;
    const int JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000;
    public static IntPtr TryAssign(int processId) {
        if (processId <= 0) return IntPtr.Zero;
        IntPtr job = CreateJobObject(IntPtr.Zero, null);
        if (job == IntPtr.Zero) return IntPtr.Zero;
        int size = IntPtr.Size == 8 ? 144 : 112;
        IntPtr info = Marshal.AllocHGlobal(size);
        try {
            for (int i = 0; i < size; i++) Marshal.WriteByte(info, i, 0);
            Marshal.WriteInt32(info, 16, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
            if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, info, (uint)size)) {
                CloseHandle(job);
                return IntPtr.Zero;
            }
        } finally {
            Marshal.FreeHGlobal(info);
        }
        IntPtr handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SET_QUOTA, false, processId);
        if (handle == IntPtr.Zero) {
            CloseHandle(job);
            return IntPtr.Zero;
        }
        try {
            if (!AssignProcessToJobObject(job, handle)) {
                CloseHandle(job);
                return IntPtr.Zero;
            }
        } finally {
            CloseHandle(handle);
        }
        return job;
    }
    public static void TerminateAndClose(IntPtr job) {
        if (job == IntPtr.Zero) return;
        TerminateJobObject(job, 1);
        CloseHandle(job);
    }
}
'@
    try {
        Add-Type -TypeDefinition $source -ErrorAction Stop
        return $true
    } catch {
        Write-Host "Could not create a kill-on-close job ($($_.Exception.Message)). taskkill /T will still run."
        return $false
    }
}

function Stop-SmokeTree {
    param($proc)
    if ($null -eq $proc) { return }
    $id = 0
    try { $id = [int]$proc.Id } catch { return }
    if ($id -le 0) { return }
    $alive = $false
    try { $alive = -not $proc.HasExited } catch { $alive = $false }
    if ($alive) {
        Write-Host "Stopping smoke process $id and its children."
        & taskkill.exe /F /T /PID $id 2>&1 | ForEach-Object { Write-Host $_ }
        try { $proc.WaitForExit(10000) | Out-Null } catch {}
    } else {
        # Parent exit does not always reap WebView2. /T on a dead pid is a no-op;
        # the job object is what kills children that were reparented.
        & taskkill.exe /F /T /PID $id 2>&1 | Out-Null
    }
}

function Invoke-StartupSmoke {
    $name = "Startup smoke: bundled app get_settings"
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

    # `cargo build --release` does not pass tauri/custom-protocol, so the exe
    # loads devUrl (http://localhost:1420) and the window shows
    # ERR_CONNECTION_REFUSED. `tauri build` enables that feature, runs
    # beforeBuildCommand, and embeds frontendDist. --no-bundle skips MSI/NSIS.
    $started = Get-Date
    & $npm.Source run tauri -- build --no-bundle
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL  $name"
        Write-Host "tauri build --no-bundle failed. A plain cargo build --release does not embed the frontend."
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

    $jobReady = Initialize-SmokeJob
    $proc = $null
    $job = [IntPtr]::Zero
    $detail = $null
    $payload = $null
    $ok = $false
    try {
        $proc = New-Object System.Diagnostics.Process
        $proc.StartInfo = $psi
        [void]$proc.Start()
        if ($jobReady) {
            $job = [CadenceSmokeJob]::TryAssign([int]$proc.Id)
            if ($job -eq [IntPtr]::Zero) {
                Write-Host "Smoke process was not placed in a kill-on-close job. taskkill /T will still run."
            }
        }
        $elapsed = [int]((Get-Date) - $started).TotalSeconds
        $runLimit = [Math]::Max(90, $limitSec - $elapsed)
        if (-not $proc.WaitForExit($runLimit * 1000)) {
            $detail = "The bundled app did not report get_settings within ${runLimit}s."
        } elseif (-not (Test-Path $result)) {
            $detail = "No smoke result (exit $($proc.ExitCode)). Bolo.com may already be running, or the settings page never invoked get_settings."
        } else {
            $raw = [System.IO.File]::ReadAllText($result).Trim()
            if (-not $raw.StartsWith("OK ")) {
                $detail = $raw
            } else {
                try {
                    $settings = $raw.Substring(3) | ConvertFrom-Json
                } catch {
                    $settings = $null
                    $detail = "get_settings did not return JSON: $raw"
                }
                if ($null -eq $detail -and ($settings.cleanupEnabled -ne $true -or $settings.model -ne "whisper-large-v3-turbo")) {
                    $detail = "Unexpected settings: $raw"
                }
                if ($null -eq $detail -and $settings.historyEnabled -ne $true) {
                    $detail = "historyEnabled should stay on when the file omits it: $raw"
                }
                if ($null -eq $detail -and $null -ne $settings.microphone) {
                    $detail = "microphone should be null: $raw"
                }
                if ($null -eq $detail -and (-not $settings.hotkey.ctrl -or -not $settings.hotkey.win)) {
                    $detail = "hotkey was not Ctrl+Win: $raw"
                }
                if ($null -eq $detail) {
                    $ok = $true
                    $payload = $raw.Substring(3)
                }
            }
        }
    } catch {
        $detail = "Could not start or watch ${exe}: $($_.Exception.Message)"
    } finally {
        if ($job -ne [IntPtr]::Zero) {
            try { [CadenceSmokeJob]::TerminateAndClose($job) } catch {}
            $job = [IntPtr]::Zero
        }
        Stop-SmokeTree $proc
    }

    if (-not $ok) {
        Write-Host "FAIL  $name"
        if ($detail) { Write-Host $detail }
        return $false
    }
    Write-Host "PASS  $name"
    Write-Host "get_settings $payload"
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

$smokeName = "Startup smoke: bundled app get_settings"
$smokeOk = Invoke-StartupSmoke
if ($smokeOk) { $passed += $smokeName } else { $failed += $smokeName }

Write-Host ""
Write-Host "========"
foreach ($name in $passed) { Write-Host "PASS  $name" }
foreach ($name in $failed) { Write-Host "FAIL  $name" }
Write-Host ("{0} passed, {1} failed" -f $passed.Count, $failed.Count)
if ($failed.Count -gt 0) { exit 1 }
exit 0
