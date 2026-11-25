<#
.SYNOPSIS
  Bumps the version, commits, and creates the annotated tag vX.Y.Z. Does not push.
.DESCRIPTION
  Cargo.toml is the version source; Cargo.lock and the README badge are kept in sync. CHANGELOG.md's Unreleased section is stamped with the version and date and must not be
  empty. Pushing the tag triggers .github/workflows/release.yml, which runs the full CI workflow on
  the tag and publishes the installer it tested as a GitHub Release.
.EXAMPLE
  ./scripts/release.ps1 -Version 0.4.0
  git push origin HEAD --follow-tags
#>
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
  if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must be plain X.Y.Z (got '$Version')." }
  if (git status --porcelain) { throw 'Working tree is not clean; commit or stash first.' }
  $tag = "v$Version"
  if (git tag --list $tag) { throw "Tag $tag already exists." }

  $cargo = Get-Content Cargo.toml -Raw
  $current = [regex]::Match($cargo, '(?m)^version = "([^"]+)"').Groups[1].Value
  if ([version]$Version -le [version]$current) { throw "Version $Version must be greater than the current $current." }

  # The Unreleased section becomes this release's notes (release.yml reads it back by version).
  & (Join-Path $PSScriptRoot 'release-notes.ps1') Unreleased | Out-Null
  $changelog = Get-Content CHANGELOG.md -Raw
  $stamped = "## [Unreleased]`n`n## [$Version] - $(Get-Date -Format 'yyyy-MM-dd')"
  $changelog = ([regex]'(?m)^## \[Unreleased\][^\n]*').Replace($changelog, $stamped, 1)
  Set-Content CHANGELOG.md $changelog -NoNewline

  $cargoManifest = Get-Content Cargo.toml -Raw
  $cargoManifest = [regex]::Replace($cargoManifest, '(?m)^version = "[^"]+"', "version = `"$Version`"", 1)
  Set-Content Cargo.toml $cargoManifest -NoNewline
  # The README's badge is the version a reader sees first; keep it in step with Cargo.toml.
  $readme = Get-Content README.md -Raw
  $readme = [regex]::Replace($readme, '(?m)^\*\*v[^*]+\*\*', "**v$Version**", 1)
  Set-Content README.md $readme -NoNewline

  cargo update --offline -p marky --quiet
  if ($LASTEXITCODE -ne 0) { throw 'cargo update failed' }

  git add Cargo.toml Cargo.lock CHANGELOG.md README.md
  git commit --quiet -m "Release $tag"
  git tag -a $tag -m "Marky $Version"
  Write-Host "Created $tag. Publish it with: git push origin HEAD --follow-tags"
}
finally { Pop-Location }
