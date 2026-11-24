# Extracts the firmware images embedded in CheckVersionPCI.exe for Desktop
# Video versions that ship it that way (DV 10.9.10 - 11.4.1: the MSI's
# Firmware\ folder only holds data-bdaa.bin; every device image lives in the
# updater exe as a PE RCDATA resource). From DV 11.5 onward the images moved to
# encrypted Firmware\data-rNN.bin files, which the harvest already extracts.
#
# Unlike harvest.ps1 this needs NO elevation and no msiexec: 7-Zip reads the
# zip -> MSI (compound file) -> embedded CAB -> PE resources directly.
#
# Output per version (beside the existing extract):
#   firmware/installers/dv-<v>-extract/Firmware/CheckVersionPCI-rcdata/rc-<id>.bin
#   .../CheckVersionPCI-rcdata/resources.csv  (id,bytes,sha256,entropy,type,head16)
#   firmware/installers/dv-<v>-inventory.csv  (regenerated: whole Firmware\ tree)
# Then run `ubmdf index rebuild`.
#
# Usage: pwsh -File tools/extract-updater-firmware.ps1 [-From 10.9.10] [-To 99] [-Force]
param(
    [string]$From = '10.9.10',
    [string]$To = '999',
    [switch]$Force,
    [string]$SevenZip = 'C:\Program Files\7-Zip\7z.exe'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$inst = Join-Path $root 'firmware/installers'
if (-not (Test-Path $SevenZip)) { $SevenZip = (Get-Command 7z -ErrorAction Stop).Source }

Add-Type -TypeDefinition @'
using System;
public static class Blob {
    public static double Entropy(byte[] b) {
        if (b.Length == 0) return 0;
        long[] c = new long[256];
        foreach (byte x in b) c[x]++;
        double e = 0, n = b.Length;
        foreach (long v in c) if (v > 0) { double p = v / n; e -= p * Math.Log(p, 2); }
        return e;
    }
    public static string Kind(byte[] b) {
        if (b.Length >= 4 && b[0] == 0x7f && b[1] == 0x45 && b[2] == 0x4c && b[3] == 0x46) return "elf";
        bool allFF = true; foreach (byte x in b) if (x != 0xFF) { allFF = false; break; }
        if (allFF) return "blank";
        int lim = Math.Min(b.Length - 3, 4096);
        for (int i = 0; i < lim; i++)
            if (b[i] == 0xAA && b[i+1] == 0x99 && b[i+2] == 0x55 && b[i+3] == 0x66) return "xilinx-bitstream";
        if (b.Length >= 16 && b[0] == 0xFF && b[1] == 0xFF && b[2] == 0xFF && b[3] == 0xFF) return "ff-padded";
        return "binary";
    }
}
'@

function Sha256Hex([string]$path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLower() }
function VKey([string]$v) { [version](($v.Split('.') + @('0', '0', '0'))[0..3] -join '.') }

$from = VKey $From; $to = VKey $To
$zips = Get-ChildItem $inst -Filter 'Blackmagic_Desktop_Video_Windows_*.zip' |
    ForEach-Object { $v = $_.BaseName -replace '^Blackmagic_Desktop_Video_Windows_', ''; [pscustomobject]@{ Version = $v; Key = (VKey $v); Zip = $_.FullName } } |
    Where-Object { $_.Key -ge $from -and $_.Key -le $to } | Sort-Object Key

$report = @()
foreach ($z in $zips) {
    $v = $z.Version
    $ext = Join-Path $inst "dv-$v-extract"
    $fw = Join-Path $ext 'Firmware'
    $dest = Join-Path $fw 'CheckVersionPCI-rcdata'
    if (-not $Force -and (Get-ChildItem $fw -Filter 'data-r*.bin' -ErrorAction SilentlyContinue)) {
        Write-Host "[$v] SKIP: encrypted data-r*.bin layout (already extracted by harvest)"
        $report += [pscustomobject]@{ version = $v; status = 'skip-data-r'; images = 0; bytes = 0 }
        continue
    }
    if (-not $Force -and (Test-Path (Join-Path $dest 'resources.csv'))) {
        Write-Host "[$v] SKIP: already extracted"
        $report += [pscustomobject]@{ version = $v; status = 'skip-done'; images = 0; bytes = 0 }
        continue
    }
    $tmp = Join-Path ([IO.Path]::GetTempPath()) "ubmdf-upd-$v"
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    New-Item -ItemType Directory -Force $tmp | Out-Null
    try {
        Write-Host "[$v] zip -> msi"
        & $SevenZip e $z.Zip -o"$tmp" '*.msi' -y | Out-Null
        $msi = Get-ChildItem $tmp -Filter *.msi | Select-Object -First 1
        if (-not $msi) { throw 'no .msi in zip' }
        Write-Host "[$v] msi -> CheckVersionPCI.exe"
        & $SevenZip e $msi.FullName -o"$tmp" 'CheckVersionPCI.exe.*' -y | Out-Null
        $exe = Get-ChildItem $tmp -Filter 'CheckVersionPCI.exe.*' | Select-Object -First 1
        if (-not $exe) {
            Write-Host "[$v] no CheckVersionPCI.exe in MSI - nothing to extract"
            $report += [pscustomobject]@{ version = $v; status = 'no-updater'; images = 0; bytes = 0 }
            continue
        }
        Remove-Item $msi.FullName -Force
        Write-Host "[$v] exe ($([math]::Round($exe.Length / 1MB)) MB) -> RCDATA"
        & $SevenZip x $exe.FullName -o"$tmp\pe" '.rsrc' -y | Out-Null
        $rc = Get-ChildItem "$tmp\pe" -Recurse -Directory | Where-Object Name -eq 'RCDATA' | Select-Object -First 1
        if (-not $rc) { throw 'no RCDATA resources in CheckVersionPCI.exe' }

        if (Test-Path $dest) { Remove-Item $dest -Recurse -Force }
        New-Item -ItemType Directory -Force $dest | Out-Null
        $rows = @(); $total = 0
        foreach ($f in (Get-ChildItem $rc.FullName -File | Sort-Object { [int]($_.Name -replace '\D', '0') })) {
            $out = Join-Path $dest "rc-$($f.Name).bin"
            Copy-Item $f.FullName $out
            $bytes = [IO.File]::ReadAllBytes($out)
            $total += $bytes.Length
            $head = -join ($bytes[0..([math]::Min(15, $bytes.Length - 1))] | ForEach-Object { '{0:x2}' -f $_ })
            $rows += [pscustomobject]@{
                id = $f.Name; bytes = $bytes.Length; sha256 = (Sha256Hex $out)
                entropy = ('{0:N3}' -f [Blob]::Entropy($bytes)); type = [Blob]::Kind($bytes); head16 = $head
            }
        }
        $rows | Export-Csv (Join-Path $dest 'resources.csv') -NoTypeInformation -Encoding utf8
        # Provenance of the source exe
        "CheckVersionPCI.exe,$($exe.Length),$(Sha256Hex $exe.FullName)" | Set-Content (Join-Path $dest 'source-exe.txt')

        # Regenerate the inventory over the whole Firmware tree (version,file,bytes,sha256)
        $invPath = Join-Path $inst "dv-$v-inventory.csv"
        $lines = @('version,file,bytes,sha256')
        foreach ($f in (Get-ChildItem $fw -Recurse -File | Where-Object { $_.Name -notin 'resources.csv', 'source-exe.txt' } | Sort-Object FullName)) {
            $rel = $f.FullName.Substring($fw.Length + 1).Replace('\', '/')
            $lines += "$v,$rel,$($f.Length),$(Sha256Hex $f.FullName)"
        }
        $lines | Set-Content $invPath -Encoding utf8
        $types = ($rows | Group-Object type | ForEach-Object { "$($_.Count) $($_.Name)" }) -join ', '
        Write-Host "[$v] OK: $($rows.Count) images, $([math]::Round($total / 1MB, 1)) MB ($types)"
        $report += [pscustomobject]@{ version = $v; status = 'ok'; images = $rows.Count; bytes = $total; types = $types }
    } catch {
        Write-Host "[$v] FAILED: $_"
        $report += [pscustomobject]@{ version = $v; status = "failed: $_"; images = 0; bytes = 0 }
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
$report | ConvertTo-Json | Set-Content (Join-Path $inst 'updater-extract-report.json')
$ok = @($report | Where-Object status -eq 'ok').Count
Write-Host "`nDone: $ok extracted, $(@($report | Where-Object status -like 'skip*').Count) skipped, $(@($report | Where-Object status -like 'failed*').Count) failed. Report: firmware/installers/updater-extract-report.json"
Write-Host 'Next: ubmdf index rebuild'
