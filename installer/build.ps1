# Builds abyss.exe (release) and the installer: dist\abyss-setup-<version>.exe
$ErrorActionPreference = "Continue"  # cargo writes progress to stderr
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
  cargo build --release
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
  $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
  $ico = Get-ChildItem target\release\build -Recurse -Filter icon.ico | Sort-Object LastWriteTime | Select-Object -Last 1
  Copy-Item $ico.FullName installer\abyss.ico -Force
  $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
  if (-not $iscc) { throw "Inno Setup 6 not found (winget install JRSoftware.InnoSetup)" }
  & $iscc "/DAppVersion=$version" installer\abyss.iss
  if ($LASTEXITCODE -ne 0) { throw "installer build failed" }
  Get-Item "dist\abyss-setup-$version.exe"
} finally { Pop-Location }
