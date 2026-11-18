<#
.SYNOPSIS
  Regenerates assets/marky.ico from app-icon.svg and app-icon-small.svg.
.DESCRIPTION
  The SVG files are the artwork; the .ico is generated and committed because build.rs embeds it.
  Run this after editing either SVG.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Push-Location $root
try {
    cargo run --quiet --example make_icon
    if ($LASTEXITCODE -ne 0) { throw "make_icon failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}
