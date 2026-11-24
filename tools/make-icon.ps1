# Generates the TEMPORARY UBDMF app icon (swap out later):
# an UltraStudio-style capture box whose front LCD shows EBU colour bars
# (100/0/75/0: white at 100%, colours at 75%), 1:1 square render.
# Output: assets/icon/ubdmf.png (512 px) + assets/icon/ubdmf.ico (16-256 px).
# Usage: pwsh -File tools/make-icon.ps1
Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$outDir = Join-Path $root 'assets/icon'
New-Item -ItemType Directory -Force $outDir | Out-Null

$S = 1024
$bmp = New-Object System.Drawing.Bitmap $S, $S, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'
$g.InterpolationMode = 'HighQualityBicubic'
$g.Clear([System.Drawing.Color]::Transparent)

function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}
function C([int]$r, [int]$gg, [int]$b, [int]$a = 255) { [System.Drawing.Color]::FromArgb($a, $r, $gg, $b) }
function Grad($x, $y, $w, $h, $c1, $c2) {
    New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.RectangleF $x, $y, $w, $h), $c1, $c2, 90.0
}

# Background tile (rounded square, deep navy)
$bg = RoundRect 24 24 976 976 210
$g.FillPath((Grad 24 24 976 976 (C 38 52 74) (C 14 19 28)), $bg)

# Soft shadow under the unit
$sh = New-Object System.Drawing.SolidBrush (C 0 0 0 90)
$g.FillEllipse($sh, 150, 700, 724, 70)

# Device body (brushed aluminium)
$bx = 112; $by = 290; $bw = 800; $bh = 430
$body = RoundRect $bx $by $bw $bh 46
$g.FillPath((Grad $bx $by $bw $bh (C 214 217 222) (C 132 137 145)), $body)
$g.DrawPath((New-Object System.Drawing.Pen (C 60 64 70), 6), $body)

# Black front panel
$fx = $bx + 30; $fy = $by + 30; $fw = $bw - 60; $fh = $bh - 60
$front = RoundRect $fx $fy $fw $fh 26
$g.FillPath((Grad $fx $fy $fw $fh (C 40 42 46) (C 16 17 19)), $front)

# LCD bezel + EBU bars (16:9)
$lx = $fx + 34; $ly = $fy + 52; $lw = 440; $lh = [int]($lw * 9 / 16)
$g.FillPath((New-Object System.Drawing.SolidBrush (C 6 6 7)), (RoundRect ($lx - 12) ($ly - 12) ($lw + 24) ($lh + 24) 12))
$bars = @((C 255 255 255), (C 191 191 0), (C 0 191 191), (C 0 191 0), (C 191 0 191), (C 191 0 0), (C 0 0 191), (C 0 0 0))
$colW = $lw / $bars.Count
for ($i = 0; $i -lt $bars.Count; $i++) {
    $g.FillRectangle((New-Object System.Drawing.SolidBrush $bars[$i]), [float]($lx + $i * $colW), [float]$ly, [float]($colW + 1), [float]$lh)
}
# Screen glare
$g.FillPolygon((New-Object System.Drawing.SolidBrush (C 255 255 255 28)), [System.Drawing.PointF[]]@(
    (New-Object System.Drawing.PointF $lx, $ly), (New-Object System.Drawing.PointF ($lx + $lw * 0.55), $ly),
    (New-Object System.Drawing.PointF ($lx + $lw * 0.25), ($ly + $lh)), (New-Object System.Drawing.PointF $lx, ($ly + $lh))))

# Control knob
$kx = $fx + $fw - 96; $ky = $fy + $fh / 2 - 6; $kr = 70
$g.FillEllipse((Grad ($kx - $kr) ($ky - $kr) (2 * $kr) (2 * $kr) (C 120 124 130) (C 44 46 50)), ($kx - $kr), ($ky - $kr), (2 * $kr), (2 * $kr))
$g.FillEllipse((Grad ($kx - 48) ($ky - 48) 96 96 (C 70 73 78) (C 150 154 160)), ($kx - 48), ($ky - 48), 96, 96)
$g.DrawLine((New-Object System.Drawing.Pen (C 230 232 236), 8), $kx, ($ky - 42), $kx, ($ky - 16))

# Buttons + status LED
for ($i = 0; $i -lt 3; $i++) {
    $g.FillEllipse((New-Object System.Drawing.SolidBrush (C 92 96 102)), ($lx + $lw + 30), ($ly + 22 + $i * 76), 34, 34)
}
$g.FillEllipse((New-Object System.Drawing.SolidBrush (C 70 220 120)), ($fx + 34), ($fy + $fh - 48), 22, 22)
$g.Dispose()

function Scaled([int]$n) {
    $b = New-Object System.Drawing.Bitmap $n, $n, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $gg = [System.Drawing.Graphics]::FromImage($b)
    $gg.InterpolationMode = 'HighQualityBicubic'; $gg.SmoothingMode = 'AntiAlias'; $gg.PixelOffsetMode = 'HighQuality'
    $gg.DrawImage($bmp, 0, 0, $n, $n); $gg.Dispose()
    return $b
}
function PngBytes($b) { $ms = New-Object System.IO.MemoryStream; $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); return , $ms.ToArray() }

$png512 = Scaled 512
$png512.Save((Join-Path $outDir 'ubdmf.png'), [System.Drawing.Imaging.ImageFormat]::Png)

# ICO container with PNG-compressed entries (supported since Windows Vista)
$sizes = 256, 128, 64, 48, 32, 24, 16
$blobs = New-Object 'System.Collections.Generic.List[byte[]]'
foreach ($n in $sizes) { $blobs.Add((PngBytes (Scaled $n))) }
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ms
$w.Write([UInt16]0); $w.Write([UInt16]1); $w.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $n = $sizes[$i]; $len = $blobs[$i].Length
    $w.Write([byte]($n % 256)); $w.Write([byte]($n % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([UInt16]1); $w.Write([UInt16]32); $w.Write([UInt32]$len); $w.Write([UInt32]$offset)
    $offset += $len
}
foreach ($b in $blobs) { $w.Write($b) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $outDir 'ubdmf.ico'), $ms.ToArray())
"icon written: $outDir (ubdmf.png 512px, ubdmf.ico $($sizes -join '/') px)"
