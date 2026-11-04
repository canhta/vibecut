$ErrorActionPreference = 'Stop'
$receipt = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'installation.json') -Raw | ConvertFrom-Json
if ($receipt.destination -ne $PSScriptRoot) { throw 'Installation path changed; inspect installation.json before continuing.' }
if ($receipt.pathAdded) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($current -split ';' | Where-Object { $_ -ne $PSScriptRoot })
    [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
}
foreach ($entry in $receipt.files.PSObject.Properties) {
    # Retain documentation and modified libraries for inspection.
    if ($entry.Name -notmatch '^[^/\\]+\.(exe|dll)$') { continue }
    $path = Join-Path $PSScriptRoot $entry.Name
    if (Test-Path -LiteralPath $path) {
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $entry.Value) { Remove-Item -LiteralPath $path }
        else { Write-Warning "Preserving modified file $($entry.Name)" }
    }
}
Write-Output 'Removed original executables and owned PATH entry; modified files, compliance files and scripts remain for inspection.'
