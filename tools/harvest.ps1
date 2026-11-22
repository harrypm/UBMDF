# harvest.ps1 — UBDMF auto-harvest: all Desktop Video versions, unattended
#
# Self-elevates (one UAC prompt), then for every Desktop Video version in BMD's
# catalog (non-SDK, Windows, 9.5.3 → 16.5): download → msiexec /a sandbox extract →
# firmware sha256 inventory → cleanup → next version. Produces harvest-report.json.
#
# Usage:
#   pwsh -File tools\harvest.ps1                    # all versions (default)
#   pwsh -File tools\harvest.ps1 -From 16.5 -To 14.0  # version range
#   pwsh -File tools\harvest.ps1 -NoZips             # discard zips after inventory
#
# Local zips in Downloads\FireFox are used instead of re-downloading when available.

param(
    [string]$From = '',
    [string]$To = '',
    [switch]$NoZips = $false,
    [string]$InstallersDir = "$PSScriptRoot\..\firmware\installers"
)

$ErrorActionPreference = 'Continue'  # skip-and-report, never halt the batch

# --- Self-elevate ---
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "Not elevated — relaunching with UAC prompt..."
    $args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath)
    if ($From) { $args += '-From', $From }
    if ($To) { $args += '-To', $To }
    if ($NoZips) { $args += '-NoZips' }
    Start-Process pwsh.exe -ArgumentList $args -Verb RunAs -Wait
    exit
}
Write-Host "=== UBDMF Firmware Harvest (elevated) ==="
Write-Host "Started: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"

# Transcript: everything below is captured to a file so a stall/crash is
# post-mortem diagnosable (the 21 Nov run died silently with no trace).
Start-Transcript -Path (Join-Path $PSScriptRoot 'harvest-transcript.log') -Append | Out-Null

$InstallersDir = (Resolve-Path $InstallersDir -ErrorAction SilentlyContinue).Path
if (-not $InstallersDir) { New-Item -ItemType Directory -Path $InstallersDir -Force | Out-Null; $InstallersDir = (Resolve-Path $InstallersDir).Path }

$report = @()
$catalogUrl = 'https://www.blackmagicdesign.com/api/support/us/downloads.json'
$registerUrl = 'https://www.blackmagicdesign.com/api/register/us/download'
$ua = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36'

# --- Fetch catalog ---
Write-Host "Fetching BMD catalog..."
$catalog = Invoke-RestMethod $catalogUrl -UserAgent $ua -TimeoutSec 60
$dvVersions = @()
foreach ($entry in $catalog.downloads) {
    $name = $entry.name.Trim()
    if ($name -notmatch '^Desktop Video ' -or $name -match 'SDK') { continue }
    $ver = $name -replace '^Desktop Video ', ''
    $winUrls = $entry.urls | Get-Member -MemberType NoteProperty | Where-Object { $_.Name -eq 'Windows' }
    if (-not $winUrls) { continue }
    $winList = $entry.urls.Windows
    if (-not $winList -or $winList.Count -eq 0 -or -not $winList[0].downloadId) { continue }
    $dvVersions += [pscustomobject]@{ Version = $ver; DownloadId = $winList[0].downloadId }
}
Write-Host "Found $($dvVersions.Count) Desktop Video versions in catalog (non-SDK, Windows)"

# --- Filter by range ---
if ($From -or $To) {
    # Simple numeric filter: convert version to a comparable number (16.5 → 16.5, 12.8.1 → 12.81)
    function VerNum([string]$v) {
        $parts = $v.Split('.')
        $n = 0.0
        for ($i = 0; $i -lt $parts.Count; $i++) {
            $n += [double]$parts[$i] / [math]::Pow(100, $i)
        }
        return [math]::Round($n, 6)
    }
    $fromN = if ($From) { VerNum $From } else { 0 }
    $toN = if ($To) { VerNum $To } else { [double]::MaxValue }
    $dvVersions = $dvVersions | Where-Object { (VerNum $_.Version) -ge $fromN -and (VerNum $_.Version) -le $toN }
    Write-Host "Filtered to $($dvVersions.Count) versions in range $From..$To"
}

# --- Local zip sources ---
$localZipDirs = @("$env:USERPROFILE\Downloads\FireFox", "$env:USERPROFILE\Desktop", "$env:USERPROFILE\Downloads")

# --- Process each version ---
$processed = 0; $succeeded = 0; $skipped = 0; $failed = 0
foreach ($dv in $dvVersions) {
    $ver = $dv.Version
    $processed++
    Write-Host "`n[$processed/$($dvVersions.Count)] DV $ver — " -NoNewline

    # Check if already inventoried
    $inventoryCsv = Join-Path $InstallersDir "dv-$ver-inventory.csv"
    if (Test-Path $inventoryCsv) {
        Write-Host "SKIP (inventory exists)"
        $skipped++
        $report += [pscustomobject]@{ version = $ver; status = 'skipped'; reason = 'inventory exists' }
        continue
    }

    # Find or download the zip
    $zipPath = Join-Path $InstallersDir "Blackmagic_Desktop_Video_Windows_$ver.zip"

    if (Test-Path $zipPath) {
        Write-Host "using existing zip"
    } else {
        # Check local dirs
        $localZip = $null
        foreach ($dir in $localZipDirs) {
            $candidate = Join-Path $dir "Blackmagic_Desktop_Video_Windows_$ver.zip"
            if (Test-Path $candidate) { $localZip = $candidate; break }
        }
        if ($localZip) {
            Write-Host "importing local zip from $localZip"
            Copy-Item $localZip $zipPath -Force
        } else {
            # Download from BMD
            Write-Host "downloading..." -NoNewline
            try {
                $regBody = @{ country = 'us'; platform = 'Windows'; product = 'Desktop Video'; policy = $true } | ConvertTo-Json -Compress
                $signedUrl = Invoke-RestMethod -Method Post -Uri "$registerUrl/$($dv.DownloadId)" `
                    -Body $regBody -ContentType 'application/json;charset=UTF-8' `
                    -UserAgent $ua -Headers @{ 'Origin' = 'https://www.blackmagicdesign.com' } -TimeoutSec 120
                if ($signedUrl -is [string] -and $signedUrl.StartsWith('http')) {
                    # TimeoutSec: a dead TCP connection would otherwise hang the
                    # whole batch forever (root cause of the 21 Nov silent stall).
                    Invoke-WebRequest -Uri $signedUrl -OutFile $zipPath -UserAgent $ua -TimeoutSec 900
                    if (-not (Test-Path $zipPath)) { throw "download produced no file" }
                    $size = [math]::Round((Get-Item $zipPath).Length / 1MB, 0)
                    Write-Host " ${size}MB"
                } else {
                    throw "unexpected signed URL response"
                }
            } catch {
                Write-Host " DOWNLOAD FAILED: $($_.Exception.Message)"
                $failed++
                $report += [pscustomobject]@{ version = $ver; status = 'failed'; reason = "download: $($_.Exception.Message)" }
                continue
            }
        }
    }

    # Extract MSI from zip
    $extractDir = Join-Path $InstallersDir "dv-$ver-extract"
    if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
    New-Item -ItemType Directory -Path $extractDir -Force | Out-Null

    try {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zipArchive = [IO.Compression.ZipFile]::OpenRead($zipPath)
        $msiEntry = $zipArchive.Entries | Where-Object { $_.Name -like '*.msi' } | Select-Object -First 1
        if (-not $msiEntry) { throw "no MSI in zip" }
        $msiPath = Join-Path $extractDir $msiEntry.Name
        [IO.Compression.ZipFileExtensions]::ExtractToFile($msiEntry, $msiPath, $true)
        $zipArchive.Dispose()
    } catch {
        Write-Host " EXTRACT FAILED: $($_.Exception.Message)"
        $failed++
        $report += [pscustomobject]@{ version = $ver; status = 'failed'; reason = "unzip: $($_.Exception.Message)" }
        if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue }
        continue
    }

    # msiexec /a (sandbox extract)
    $sandboxDir = Join-Path $extractDir "sandbox"
    New-Item -ItemType Directory -Path $sandboxDir -Force | Out-Null
    $msiProc = Start-Process msiexec.exe -ArgumentList '/a', "`"$msiPath`"", '/qn', "TARGETDIR=`"$sandboxDir`"" -Wait -PassThru
    if ($msiProc.ExitCode -ne 0) {
        Write-Host " MSIEXEC FAILED (exit $($msiProc.ExitCode))"
        $failed++
        $report += [pscustomobject]@{ version = $ver; status = 'failed'; reason = "msiexec exit $($msiProc.ExitCode)" }
        Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue
        continue
    }

    # Find firmware dir
    $fwDir = Get-ChildItem $sandboxDir -Recurse -Directory -Filter 'Firmware' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $fwDir) {
        # LEGACY layout (DV <= 10.9.7, verified by MSI inspection 21 Nov): there is
        # no separate Firmware directory — the firmware ships inside the drivers
        # (.sys / .cpl / plugins). Inventory the complete installer payload as the
        # archival record; the kept zip remains the authoritative copy. The sandbox
        # is discarded afterwards (the payload would double disk usage for no gain).
        Write-Host " LEGACY LAYOUT — full payload inventory"
        $allFiles = @(Get-ChildItem $sandboxDir -Recurse -File | Sort-Object FullName)
        $csvLines = @('version,file,bytes,sha256')
        $payloadTotal = 0
        foreach ($f in $allFiles) {
            $sha = (Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLower()
            $payloadTotal += $f.Length
            $relPath = $f.FullName.Substring($sandboxDir.Length + 1)
            $csvLines += "$ver,$relPath,$($f.Length),$sha"
        }
        $csvLines | Set-Content $inventoryCsv -Encoding UTF8
        Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue
        $payloadMB = [math]::Round($payloadTotal / 1MB, 1)
        Write-Host " OK (legacy: $($allFiles.Count) files, $payloadMB MB payload inventoried)"
        $succeeded++
        $report += [pscustomobject]@{ version = $ver; status = 'ok-legacy'; reason = "legacy payload: $($allFiles.Count) files, $payloadMB MB (no Firmware dir; firmware in drivers)" }
        continue
    }

    # Inventory firmware
    $fwFiles = Get-ChildItem $fwDir.FullName -Filter 'data-*.bin' | Sort-Object Name
    $csvLines = @('version,file,bytes,sha256')
    $fwTotal = 0
    foreach ($f in $fwFiles) {
        $sha = (Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLower()
        $fwTotal += $f.Length
        $csvLines += "$ver,$($f.Name),$($f.Length),$sha"
    }
    $csvLines | Set-Content $inventoryCsv -Encoding UTF8

    # Move firmware to extract root, clean sandbox + MSI
    $fwDest = Join-Path $extractDir "Firmware"
    if (-not (Test-Path $fwDest)) { Move-Item $fwDir.FullName $fwDest }
    Remove-Item $sandboxDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $msiPath -Force -ErrorAction SilentlyContinue

    # Optionally remove the zip
    if ($NoZips) { Remove-Item $zipPath -Force -ErrorAction SilentlyContinue }

    $fwMB = [math]::Round($fwTotal / 1MB, 1)
    Write-Host " OK ($(@($fwFiles).Count) firmware files, $fwMB MB)"
    $succeeded++
    $report += [pscustomobject]@{ version = $ver; status = 'ok'; reason = "$(@($fwFiles).Count) files, $fwMB MB" }
}

# --- Rebuild index ---
Write-Host "`nRebuilding index..."
Push-Location "$PSScriptRoot\.."
$ubdmf = ".\target\release\ubdmf.exe"
if (Test-Path $ubdmf) {
    & $ubdmf index rebuild 2>&1 | Select-Object -Last 3
}
Pop-Location

# --- Write harvest report ---
$reportPath = Join-Path $InstallersDir "harvest-report.json"
$reportObj = @{
    started = (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')
    total_versions = $dvVersions.Count
    processed = $processed
    succeeded = $succeeded
    skipped = $skipped
    failed = $failed
    results = $report
}
$reportObj | ConvertTo-Json -Depth 4 | Set-Content $reportPath -Encoding UTF8

Write-Host "`n=== Harvest Complete ==="
Write-Host "Processed: $processed | Succeeded: $succeeded | Skipped: $skipped | Failed: $failed"
Write-Host "Report: $reportPath"
Write-Host "Finished: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Stop-Transcript | Out-Null

# --- Print failures if any ---
if ($failed -gt 0) {
    Write-Host "`nFailed versions:"
    $report | Where-Object { $_.status -eq 'failed' } | ForEach-Object {
        Write-Host "  $($_.version): $($_.reason)"
    }
}
