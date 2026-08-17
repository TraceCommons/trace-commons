#Requires -Version 5.1
<#
.SYNOPSIS
  Draws the Trace Commons brand mark into the MSIX visual assets.

.DESCRIPTION
  A direct transcription of `.brand-mark` from the community site, the same
  one macos/Sources/TraceCommonsApp/Views/DesignSystem.swift and
  crates/trace-commons-contributor-gtk/src/ui/style.css already carry:

    background-color: #ffffff
    linear-gradient(135deg, #178f70 0 38%, transparent 38% 100%)
    linear-gradient(45deg, transparent 0 45%, #315fba 45% 100%)
    1px border #d9dfdc

  The CSS layer order puts the green wedge on top, so it is painted last.
  Light-mode colours only: Windows composes tile and taskbar icons over its
  own backplate and never asks the asset for a dark variant.

  Regenerate with:
    powershell -ExecutionPolicy Bypass -File windows/scripts/make-app-icons.ps1
#>
[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\src\TraceCommons.App\Images')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$surface = [System.Drawing.Color]::FromArgb(255, 0xFF, 0xFF, 0xFF)
$green   = [System.Drawing.Color]::FromArgb(255, 0x17, 0x8F, 0x70)
$blue    = [System.Drawing.Color]::FromArgb(255, 0x31, 0x5F, 0xBA)
$line    = [System.Drawing.Color]::FromArgb(255, 0xD9, 0xDF, 0xDC)

function Write-BrandMark {
    param(
        [Parameter(Mandatory = $true)][int]$Size,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $bmp = [System.Drawing.Bitmap]::new(
        $Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
            $g.Clear($surface)

            $s = [float]$Size

            # 45deg gradient, blue from 45% to 100%: everything on the
            # top-right side of the line y = x + 0.1s.
            #
            # Note: these points are built with [PointF]::new(...), not
            # `New-Object System.Drawing.PointF(...)`. The New-Object form
            # parses arguments in PowerShell's command/argument mode, where
            # a bare `*` inside the parens is not reliably treated as
            # multiplication -- it throws "System.Object[] does not contain
            # a method named 'op_Multiply'" here. The static ::new(...) call
            # is expression mode and has no such ambiguity.
            $bluePoly = @(
                [System.Drawing.PointF]::new(0.0,        0.0),
                [System.Drawing.PointF]::new($s,         0.0),
                [System.Drawing.PointF]::new($s,         $s),
                [System.Drawing.PointF]::new($s * 0.9,   $s),
                [System.Drawing.PointF]::new(0.0,        $s * 0.1)
            )
            $blueBrush = [System.Drawing.SolidBrush]::new($blue)
            try { $g.FillPolygon($blueBrush, [System.Drawing.PointF[]]$bluePoly) }
            finally { $blueBrush.Dispose() }

            # 135deg gradient, green from 0 to 38%: the top-left triangle cut
            # at 38% of the diagonal, so legs of 0.76s. Painted last because
            # it is the first CSS background layer, and the first layer is on
            # top.
            $greenPoly = @(
                [System.Drawing.PointF]::new(0.0,       0.0),
                [System.Drawing.PointF]::new($s * 0.76, 0.0),
                [System.Drawing.PointF]::new(0.0,       $s * 0.76)
            )
            $greenBrush = [System.Drawing.SolidBrush]::new($green)
            try { $g.FillPolygon($greenBrush, [System.Drawing.PointF[]]$greenPoly) }
            finally { $greenBrush.Dispose() }

            $pen = [System.Drawing.Pen]::new($line, 1.0)
            try { $g.DrawRectangle($pen, 0, 0, $Size - 1, $Size - 1) }
            finally { $pen.Dispose() }
        }
        finally { $g.Dispose() }

        $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $bmp.Dispose() }
}

$dir = (New-Item -ItemType Directory -Force -Path $OutputDirectory).FullName

# Exactly the three the manifest references. Extra scale variants are not
# authored: MakePri accepts a single unscaled asset and Windows downsamples,
# and every extra file is another thing that can drift from the brand.
Write-BrandMark -Size 44  -Path (Join-Path $dir 'Square44x44Logo.png')
Write-BrandMark -Size 150 -Path (Join-Path $dir 'Square150x150Logo.png')
Write-BrandMark -Size 50  -Path (Join-Path $dir 'StoreLogo.png')

Write-Host "Wrote 3 assets to $dir"
