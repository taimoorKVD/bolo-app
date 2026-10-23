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
                if ($null -eq $detail -and $settings.offlineModel -ne "base.en") {
                    $detail = "offlineModel should default to base.en when the file omits it: $raw"
                }
                if ($null -eq $detail -and ($settings.pasteHotkey.ctrl -ne $true -or $settings.pasteHotkey.win -ne $true -or $settings.pasteHotkey.key -ne "v")) {
                    $detail = "pasteHotkey should default to Ctrl+Win+V when the file omits it: $raw"
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

function Initialize-IconHash {
    if ("BoloIconHash" -as [type]) { return $true }
    $source = @'
using System;
using System.Collections.Generic;
using System.Security.Cryptography;
public static class BoloIconHash {
    static int U16(byte[] d, int o) { return d[o] | (d[o + 1] << 8); }
    static uint U32(byte[] d, int o) {
        return (uint)d[o] | ((uint)d[o + 1] << 8) | ((uint)d[o + 2] << 16) | ((uint)d[o + 3] << 24);
    }
    static string Sha(byte[] blob) {
        using (var sha = SHA256.Create()) {
            return BitConverter.ToString(sha.ComputeHash(blob)).Replace("-", "").ToLowerInvariant();
        }
    }
    public static int[] IcoSizes(byte[] data) {
        if (U16(data, 2) != 1) throw new InvalidOperationException("icon.ico is not an icon directory");
        int count = U16(data, 4);
        int[] sizes = new int[count];
        for (int i = 0; i < count; i++) {
            int width = data[6 + i * 16];
            sizes[i] = width == 0 ? 256 : width;
        }
        return sizes;
    }
    public static string[] IcoHashes(byte[] data) {
        var images = IcoImages(data);
        var hashes = new string[images.Length];
        for (int i = 0; i < images.Length; i++) hashes[i] = images[i].Sha;
        return hashes;
    }
    public static BoloIconImage[] IcoImages(byte[] data) {
        if (U16(data, 2) != 1) throw new InvalidOperationException("icon.ico is not an icon directory");
        int count = U16(data, 4);
        var images = new BoloIconImage[count];
        for (int i = 0; i < count; i++) {
            int entry = 6 + i * 16;
            int width = data[entry];
            int nbytes = (int)U32(data, entry + 8);
            int offset = (int)U32(data, entry + 12);
            byte[] blob = new byte[nbytes];
            Buffer.BlockCopy(data, offset, blob, 0, nbytes);
            images[i] = new BoloIconImage();
            images[i].Width = width == 0 ? 256 : width;
            images[i].Bytes = blob;
            images[i].Sha = Sha(blob);
        }
        return images;
    }
    public static string IcoProblem(byte[] data) {
        int[] want = new int[] { 32, 16, 24, 48, 64, 128, 256 };
        var images = IcoImages(data);
        if (images.Length != want.Length) return "icon.ico has " + images.Length + " images, expected " + want.Length;
        for (int i = 0; i < want.Length; i++) {
            if (images[i].Width != want[i]) return "icon.ico image " + i + " is " + images[i].Width + "px, expected " + want[i] + "px";
            byte[] blob = images[i].Bytes;
            if (blob.Length >= 8 && blob[0] == 0x89 && blob[1] == 0x50 && blob[2] == 0x4E && blob[3] == 0x47) {
                return images[i].Width + "px image is a PNG. ExtractAssociatedIcon would show an unrelated 32px icon.";
            }
            if (blob.Length < 40) return images[i].Width + "px image is too small to be a bitmap";
            int biSize = (int)U32(blob, 0);
            int biWidth = blob[4] | (blob[5] << 8) | (blob[6] << 16) | (blob[7] << 24);
            int biHeight = blob[8] | (blob[9] << 8) | (blob[10] << 16) | (blob[11] << 24);
            int biBit = blob[14] | (blob[15] << 8);
            if (biSize != 40 || biBit != 32 || biWidth != images[i].Width || biHeight != images[i].Width * 2) {
                return images[i].Width + "px image is not a 32-bit DIB";
            }
        }
        return null;
    }
    public static bool SameIcon(byte[] embedded, byte[] expected) {
        if (embedded == null || expected == null || embedded.Length < expected.Length) return false;
        for (int i = 0; i < expected.Length; i++) {
            if (embedded[i] != expected[i]) return false;
        }
        for (int i = expected.Length; i < embedded.Length; i++) {
            if (embedded[i] != 0) return false;
        }
        return true;
    }
    static int RvaToOff(uint rva, uint[] va, uint[] vsz, uint[] raw, uint[] rsz) {
        for (int i = 0; i < va.Length; i++) {
            uint span = vsz[i] > rsz[i] ? vsz[i] : rsz[i];
            if (rva >= va[i] && rva < va[i] + span) return (int)(raw[i] + (rva - va[i]));
        }
        throw new InvalidOperationException("icon resource RVA is outside the executable sections");
    }
    static void ReadDir(byte[] data, int off, out uint[] names, out uint[] values) {
        int n = U16(data, off + 12) + U16(data, off + 14);
        names = new uint[n];
        values = new uint[n];
        int p = off + 16;
        for (int i = 0; i < n; i++) {
            names[i] = U32(data, p);
            values[i] = U32(data, p + 4);
            p += 8;
        }
    }
    public static string[] PeIconHashes(byte[] data) {
        if (data.Length < 64 || data[0] != (byte)'M' || data[1] != (byte)'Z') {
            throw new InvalidOperationException("file is not a Windows executable");
        }
        int lfanew = (int)U32(data, 0x3C);
        if (data[lfanew] != (byte)'P' || data[lfanew + 1] != (byte)'E') {
            throw new InvalidOperationException("file has no PE header");
        }
        int coff = lfanew + 4;
        int nsections = U16(data, coff + 2);
        int optSize = U16(data, coff + 16);
        int opt = coff + 20;
        int magic = U16(data, opt);
        int ddOff = magic == 0x20B ? opt + 112 : magic == 0x10B ? opt + 96 : -1;
        if (ddOff < 0) throw new InvalidOperationException("unknown PE optional header");
        uint resRva = U32(data, ddOff + 16);
        if (resRva == 0) throw new InvalidOperationException("executable has no resource directory");
        var va = new uint[nsections];
        var vsz = new uint[nsections];
        var raw = new uint[nsections];
        var rsz = new uint[nsections];
        int sec = opt + optSize;
        for (int i = 0; i < nsections; i++) {
            int o = sec + i * 40;
            vsz[i] = U32(data, o + 8);
            va[i] = U32(data, o + 12);
            rsz[i] = U32(data, o + 16);
            raw[i] = U32(data, o + 20);
        }
        int resBase = RvaToOff(resRva, va, vsz, raw, rsz);
        uint[] types, typeVals;
        ReadDir(data, resBase, out types, out typeVals);
        var hashes = new List<string>();
        for (int i = 0; i < types.Length; i++) {
            if (types[i] != 3 || (typeVals[i] & 0x80000000) == 0) continue;
            uint[] ids, idVals;
            ReadDir(data, resBase + (int)(typeVals[i] & 0x7FFFFFFF), out ids, out idVals);
            for (int j = 0; j < idVals.Length; j++) {
                if ((idVals[j] & 0x80000000) == 0) continue;
                uint[] langs, langVals;
                ReadDir(data, resBase + (int)(idVals[j] & 0x7FFFFFFF), out langs, out langVals);
                for (int k = 0; k < langVals.Length; k++) {
                    if ((langVals[k] & 0x80000000) != 0) continue;
                    int leaf = resBase + (int)langVals[k];
                    int start = RvaToOff(U32(data, leaf), va, vsz, raw, rsz);
                    int size = (int)U32(data, leaf + 4);
                    byte[] blob = new byte[size];
                    Buffer.BlockCopy(data, start, blob, 0, size);
                    hashes.Add(Sha(blob));
                }
            }
        }
        return hashes.ToArray();
    }
    public static BoloIconImage[] PePrimaryIcons(byte[] data) {
        if (data.Length < 64 || data[0] != (byte)'M' || data[1] != (byte)'Z') {
            throw new InvalidOperationException("file is not a Windows executable");
        }
        int lfanew = (int)U32(data, 0x3C);
        if (data[lfanew] != (byte)'P' || data[lfanew + 1] != (byte)'E') {
            throw new InvalidOperationException("file has no PE header");
        }
        int coff = lfanew + 4;
        int nsections = U16(data, coff + 2);
        int optSize = U16(data, coff + 16);
        int opt = coff + 20;
        int magic = U16(data, opt);
        int ddOff = magic == 0x20B ? opt + 112 : magic == 0x10B ? opt + 96 : -1;
        if (ddOff < 0) throw new InvalidOperationException("unknown PE optional header");
        uint resRva = U32(data, ddOff + 16);
        if (resRva == 0) throw new InvalidOperationException("executable has no resource directory");
        var va = new uint[nsections];
        var vsz = new uint[nsections];
        var raw = new uint[nsections];
        var rsz = new uint[nsections];
        int sec = opt + optSize;
        for (int i = 0; i < nsections; i++) {
            int o = sec + i * 40;
            vsz[i] = U32(data, o + 8);
            va[i] = U32(data, o + 12);
            rsz[i] = U32(data, o + 16);
            raw[i] = U32(data, o + 20);
        }
        int resBase = RvaToOff(resRva, va, vsz, raw, rsz);
        uint[] types, typeVals;
        ReadDir(data, resBase, out types, out typeVals);
        var icons = new Dictionary<int, byte[]>();
        byte[] group = null;
        int bestGroupId = int.MaxValue;
        for (int i = 0; i < types.Length; i++) {
            uint type = types[i];
            if ((type != 3 && type != 14) || (typeVals[i] & 0x80000000) == 0) continue;
            uint[] ids, idVals;
            ReadDir(data, resBase + (int)(typeVals[i] & 0x7FFFFFFF), out ids, out idVals);
            for (int j = 0; j < idVals.Length; j++) {
                if ((idVals[j] & 0x80000000) == 0) continue;
                uint[] langs, langVals;
                ReadDir(data, resBase + (int)(idVals[j] & 0x7FFFFFFF), out langs, out langVals);
                if ((ids[j] & 0x80000000) != 0) continue;
                if (langVals.Length == 0 || (langVals[0] & 0x80000000) != 0) continue;
                int leaf = resBase + (int)langVals[0];
                int start = RvaToOff(U32(data, leaf), va, vsz, raw, rsz);
                int size = (int)U32(data, leaf + 4);
                byte[] blob = new byte[size];
                Buffer.BlockCopy(data, start, blob, 0, size);
                int id = (int)ids[j];
                if (type == 3) icons[id] = blob;
                else if (id < bestGroupId) {
                    bestGroupId = id;
                    group = blob;
                }
            }
        }
        if (group == null || group.Length < 6) throw new InvalidOperationException("executable has no icon group");
        int count = U16(group, 4);
        var images = new List<BoloIconImage>();
        for (int n = 0; n < count; n++) {
            int entry = 6 + n * 14;
            if (entry + 14 > group.Length) break;
            int width = group[entry];
            int id = U16(group, entry + 12);
            byte[] blob;
            if (!icons.TryGetValue(id, out blob)) {
                throw new InvalidOperationException("icon group points at missing image " + id);
            }
            var image = new BoloIconImage();
            image.Width = width == 0 ? 256 : width;
            image.Bytes = blob;
            image.Sha = Sha(blob);
            images.Add(image);
        }
        return images.ToArray();
    }
}
public class BoloIconImage {
    public int Width;
    public byte[] Bytes;
    public string Sha;
}
'@
    try {
        Add-Type -TypeDefinition $source -ErrorAction Stop
        return $true
    } catch {
        Write-Host "Could not compile the icon reader ($($_.Exception.Message))."
        return $false
    }
}

function Test-DisplayedIcon {
    param(
        [string]$Label,
        [string]$Path,
        [object[]]$Expected
    )
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    try {
        $group = @([BoloIconHash]::PePrimaryIcons($bytes))
    } catch {
        Write-Host "$Label icon could not be read ($($_.Exception.Message))."
        return $false
    }
    if ($group.Count -eq 0) {
        Write-Host "$Label has no icon for Windows to display."
        return $false
    }
    $ok = $true
    foreach ($want in $Expected) {
        $matched = $false
        foreach ($got in $group) {
            if ($got.Width -eq $want.Width -and [BoloIconHash]::SameIcon($got.Bytes, $want.Bytes)) {
                $matched = $true
                break
            }
        }
        if (-not $matched) {
            Write-Host "$Label does not display the $($want.Width)px Bolo.com icon."
            $ok = $false
        }
    }
    foreach ($got in $group) {
        $known = $false
        foreach ($want in $Expected) {
            if ([BoloIconHash]::SameIcon($got.Bytes, $want.Bytes)) {
                $known = $true
                break
            }
        }
        if (-not $known) {
            Write-Host "$Label is showing a $($got.Width)px image that is not the Bolo.com icon."
            $ok = $false
        }
    }
    if ($ok) {
        $widths = ($group | ForEach-Object { $_.Width }) -join ", "
        Write-Host "$Label displays the Bolo.com icon ($widths)."
    }
    return $ok
}

function Invoke-EmbeddedIcons {
    $name = "Embedded app and setup icon"
    $limitSec = 1200
    Write-Host ""
    Write-Host "---- $name (limit ${limitSec}s) ----"

    $ico = Join-Path $root "src-tauri\icons\icon.ico"
    if (-not (Test-Path $ico)) {
        Write-Host "FAIL  $name"
        Write-Host "Missing $ico"
        return $false
    }
    if (-not (Initialize-IconHash)) {
        Write-Host "FAIL  $name"
        return $false
    }
    $icoBytes = [System.IO.File]::ReadAllBytes($ico)
    $problem = [BoloIconHash]::IcoProblem($icoBytes)
    if ($problem) {
        Write-Host "FAIL  $name"
        Write-Host $problem
        return $false
    }
    $expected = @([BoloIconHash]::IcoImages($icoBytes))

    $npm = Get-Command npm.cmd -ErrorAction SilentlyContinue
    if (-not $npm) { $npm = Get-Command npm -ErrorAction SilentlyContinue }
    if (-not $npm) {
        Write-Host "FAIL  $name"
        Write-Host "npm was not found on PATH."
        return $false
    }

    # npm.cmd is a batch file. UseShellExecute=false cannot launch it, so go through cmd.exe.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = Join-Path $env:SystemRoot "System32\cmd.exe"
    $psi.Arguments = "/c `"$($npm.Source)`" run tauri -- build --bundles nsis"
    $psi.WorkingDirectory = $root
    $psi.UseShellExecute = $false
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    try {
        [void]$proc.Start()
    } catch {
        Write-Host "FAIL  $name"
        Write-Host "Could not start the NSIS build: $($_.Exception.Message)"
        return $false
    }
    $limitMs = [Math]::Min($limitSec * 1000, [int]::MaxValue)
    if (-not $proc.WaitForExit($limitMs)) {
        Write-Host "FAIL  $name"
        Write-Host "NSIS build timed out after ${limitSec}s."
        & taskkill.exe /F /T /PID $proc.Id 2>&1 | ForEach-Object { Write-Host $_ }
        try { $proc.WaitForExit(10000) | Out-Null } catch {}
        return $false
    }
    if ($proc.ExitCode -ne 0) {
        Write-Host "FAIL  $name"
        Write-Host "tauri build --bundles nsis failed (exit $($proc.ExitCode))."
        return $false
    }

    $exe = Join-Path $root "src-tauri\target\release\cadence.exe"
    $nsisDir = Join-Path $root "src-tauri\target\release\bundle\nsis"
    $setup = Get-ChildItem -Path $nsisDir -Filter "*setup.exe" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not (Test-Path $exe) -or $null -eq $setup) {
        Write-Host "FAIL  $name"
        Write-Host "Expected $exe and an NSIS *setup.exe under $nsisDir."
        return $false
    }
    $appOk = Test-DisplayedIcon "cadence.exe" $exe $expected
    $setupOk = Test-DisplayedIcon "setup exe" $setup.FullName $expected
    if ($appOk -and $setupOk) {
        Write-Host "PASS  $name"
        return $true
    }
    Write-Host "FAIL  $name"
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

$smokeName = "Startup smoke: bundled app get_settings"
$smokeOk = Invoke-StartupSmoke
if ($smokeOk) { $passed += $smokeName } else { $failed += $smokeName }

$iconName = "Embedded app and setup icon"
$iconOk = Invoke-EmbeddedIcons
if ($iconOk) { $passed += $iconName } else { $failed += $iconName }

Write-Host ""
Write-Host "========"
foreach ($name in $passed) { Write-Host "PASS  $name" }
foreach ($name in $failed) { Write-Host "FAIL  $name" }
Write-Host ("{0} passed, {1} failed" -f $passed.Count, $failed.Count)
if ($failed.Count -gt 0) { exit 1 }
exit 0
