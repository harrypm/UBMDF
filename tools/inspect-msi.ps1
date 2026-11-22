# inspect-msi.ps1 — one-off: dump the msiexec /a layout of old DV versions
# whose harvest failed with "no Firmware directory" (9.5.3 -> 11.3).
# Self-elevates (one UAC), writes tools/inspect-msi.log, exits.
param(
    [string[]]$Versions = @('11.3', '10.5', '9.5.3'),
    [string]$InstallersDir = "$PSScriptRoot\..\firmware\installers"
)

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Start-Process pwsh.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath) -Verb RunAs -Wait
    exit
}

$log = Join-Path $PSScriptRoot 'inspect-msi.log'
"=== MSI layout inspection $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') ===" | Set-Content $log

foreach ($ver in $Versions) {
    $zipPath = Join-Path $InstallersDir "Blackmagic_Desktop_Video_Windows_$ver.zip"
    "" | Add-Content $log
    "########## DV $ver ##########" | Add-Content $log
    if (-not (Test-Path $zipPath)) { "zip missing: $zipPath" | Add-Content $log; continue }

    $work = Join-Path $env:TEMP "ubdmf-inspect-$ver"
    if (Test-Path $work) { Remove-Item $work -Recurse -Force }
    $zipDir = Join-Path $work 'zip'
    $sandbox = Join-Path $work 'sandbox'
    New-Item -ItemType Directory -Force $zipDir | Out-Null
    New-Item -ItemType Directory -Force $sandbox | Out-Null

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zipArchive = [IO.Compression.ZipFile]::OpenRead($zipPath)
    "--- zip entries ---" | Add-Content $log
    foreach ($e in $zipArchive.Entries) {
        "  $($e.Name)  ($([math]::Round($e.Length/1MB,2)) MB)" | Add-Content $log
    }
    $msiEntry = $zipArchive.Entries | Where-Object { $_.Name -like '*.msi' } | Select-Object -First 1
    if (-not $msiEntry) { "no MSI in zip" | Add-Content $log; continue }
    $msiPath = Join-Path $zipDir $msiEntry.Name
    [IO.Compression.ZipFileExtensions]::ExtractToFile($msiEntry, $msiPath, $true)
    $zipArchive.Dispose()

    # temp layout has no spaces, so plain unquoted args are safe here
    $p = Start-Process msiexec.exe -ArgumentList '/a', $msiPath, '/qn', "TARGETDIR=$sandbox" -Wait -PassThru
    "msiexec exit: $($p.ExitCode)" | Add-Content $log

    "--- directories ---" | Add-Content $log
    $dirs = Get-ChildItem $sandbox -Recurse -Directory -ErrorAction SilentlyContinue
    foreach ($d in ($dirs | Select-Object -First 60)) {
        "  DIR  $($d.FullName)" | Add-Content $log
    }
    "  (total dirs: $(@($dirs).Count))" | Add-Content $log

    "--- candidate firmware files (non exe/dll/doc) ---" | Add-Content $log
    $files = Get-ChildItem $sandbox -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -notin '.exe', '.dll', '.chm', '.txt', '.rtf', '.ini', '.xml', '.ico', '.png', '.bmp', '.cat', '.inf', '.config', '.manifest' }
    foreach ($f in ($files | Select-Object -First 80)) {
        "  $($f.Name)  ($([math]::Round($f.Length/1MB,2)) MB)  @ $($f.DirectoryName)" | Add-Content $log
    }
    "  (total candidates: $(@($files).Count))" | Add-Content $log

    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}

"" | Add-Content $log
"=== done ===" | Add-Content $log
