# Install-Pair from install.ps1: a failed swap puts the previous pair back; a good one replaces
# both. Run on Windows CI: pwsh -NoProfile -File scripts/install-ps1.test.ps1
$ErrorActionPreference = "Stop"
$env:FIGLENS_INSTALL_DEFINE_ONLY = "1"
. "$PSScriptRoot\install.ps1"
$Names = @("figlens-embed.exe", "figlens.exe")

function Setup {
    $root = Join-Path ([IO.Path]::GetTempPath()) ("figlens-install-test-" + [Guid]::NewGuid())
    $from = Join-Path $root "out"; $bin = Join-Path $root "bin"
    New-Item -ItemType Directory -Path $from, $bin | Out-Null
    foreach ($n in $Names) { Set-Content (Join-Path $from $n) "new"; Set-Content (Join-Path $bin $n) "old" }
    return @($from, $bin)
}

function Check($bin, $want) {
    foreach ($n in $Names) {
        $got = (Get-Content (Join-Path $bin $n)).Trim()
        if ($got -ne $want) { throw "$n holds '$got', expected '$want'" }
    }
    # Compare names in PowerShell: a native -Filter "*.exe.*" also matches the bare .exe on Windows.
    $left = @(Get-ChildItem $bin | Where-Object { $Names -notcontains $_.Name })
    if ($left.Count) { throw "left behind: $($left.Name -join ', ')" }
}

# The second exe's replacement fails after the first was swapped: both keep the old copies.
$from, $bin = Setup
$script:Moves = 0
function Move-Item {
    param($Path, $Destination, [switch]$Force, $ErrorAction)
    if ($Path -like "*.new") {
        $script:Moves++
        if ($script:Moves -eq 2) { throw "injected move failure" }
    }
    Microsoft.PowerShell.Management\Move-Item -Path $Path -Destination $Destination -Force
}
$failed = $false
try { Install-Pair $from $bin $Names } catch { $failed = $true }
if (-not $failed) { throw "a failed swap reported success" }
Check $bin "old"
Remove-Item Function:\Move-Item

# Without a failure both are replaced.
$from, $bin = Setup
Install-Pair $from $bin $Names
Check $bin "new"
Write-Host "install.ps1 swap: ok"
