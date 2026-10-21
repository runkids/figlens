#Requires -Version 5.1
<#
.SYNOPSIS
    Install the latest figlens release on Windows.
.EXAMPLE
    irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 | iex
#>

$ErrorActionPreference = "Stop"
# PowerShell 5.1 defaults to TLS 1.0/1.1; GitHub requires TLS 1.2+.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = "runkids/figlens"

function Fail { param($Message) Write-Host "figlens: $Message" -ForegroundColor Red; exit 1 }

switch ($env:PROCESSOR_ARCHITECTURE) {
    # Windows on ARM runs the x64 build under emulation.
    { $_ -in "AMD64", "ARM64" } { $Target = "x86_64-pc-windows-gnu" }
    default { Fail "unsupported CPU $env:PROCESSOR_ARCHITECTURE" }
}

# The latest tag from the releases/latest redirect, which has no API rate limit.
try {
    $response = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" `
        -MaximumRedirection 0 -UseBasicParsing -ErrorAction SilentlyContinue
} catch {
    # PowerShell 5.1 throws on 3xx; the response is on the exception.
    $response = $_.Exception.Response
}
$Tag = $null
if ($response -and $response.Headers -and "$($response.Headers["Location"])" -match "/tag/([^/\s]+)") {
    $Tag = $Matches[1]
} else {
    try { $Tag = (Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest").tag_name }
    catch { Fail "no release found at https://github.com/$Repo/releases" }
}
$Version = $Tag.TrimStart("v")

$BinDir = if ($env:FIGLENS_BIN_DIR) { $env:FIGLENS_BIN_DIR } else { "$env:LOCALAPPDATA\Programs\figlens" }
$Archive = "figlens-$Version-$Target.zip"
$Base = "https://github.com/$Repo/releases/download/$Tag"
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("figlens-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    Write-Host "Downloading figlens $Version for $Target..."
    Invoke-WebRequest -Uri "$Base/$Archive" -OutFile "$Tmp\$Archive" -UseBasicParsing
    Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile "$Tmp\SHA256SUMS" -UseBasicParsing
    $want = (Get-Content "$Tmp\SHA256SUMS" | Where-Object { $_ -match [regex]::Escape($Archive) + '$' } |
        ForEach-Object { ($_ -split '\s+')[0] }) | Select-Object -First 1
    $got = (Get-FileHash "$Tmp\$Archive" -Algorithm SHA256).Hash.ToLower()
    if (-not $want -or $want -ne $got) { Fail "$Archive does not match the release's SHA256SUMS" }
    Expand-Archive -Path "$Tmp\$Archive" -DestinationPath "$Tmp\out" -Force

    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
    $Exe = Join-Path $BinDir "figlens.exe"
    # A running exe cannot be overwritten, but it can be renamed (figlens update replaces itself).
    if (Test-Path $Exe) { Move-Item $Exe "$Exe.old" -Force }
    Move-Item "$Tmp\out\figlens.exe" $Exe -Force
    Remove-Item "$Exe.old" -Force -ErrorAction SilentlyContinue
} finally {
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($UserPath -split ";") -notcontains $BinDir) {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$BinDir", "User")
    $env:Path = "$env:Path;$BinDir"
    Write-Host "Added $BinDir to your PATH; open a new terminal to use it everywhere." -ForegroundColor Yellow
}
Write-Host "installed $(& $Exe --version) to $BinDir" -ForegroundColor Green
