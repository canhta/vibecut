param([string]$Destination = "$env:LOCALAPPDATA\Programs\VibeCut")
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Destination) { throw 'Destination exists; uninstall the previous version or choose a new directory.' }
$Destination = [IO.Path]::GetFullPath($Destination)
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'install-manifest.json') -Raw | ConvertFrom-Json
foreach ($entry in $manifest.PSObject.Properties) {
    $source = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot $entry.Name))
    if (-not $source.StartsWith($PSScriptRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid manifest path' }
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $entry.Value) { throw "Checksum mismatch: $($entry.Name)" }
}
$oldPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$added = @($oldPath -split ';') -notcontains $Destination
try {
    New-Item -ItemType Directory -Path $Destination | Out-Null
    foreach ($entry in $manifest.PSObject.Properties) {
        if ($entry.Name -eq 'install.ps1') { continue }
        $target = Join-Path $Destination $entry.Name
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $entry.Name) -Destination $target
    }
    @{ pathAdded = $added; destination = $Destination; files = $manifest } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Destination 'installation.json')
    if ($added) { [Environment]::SetEnvironmentVariable('Path', "$oldPath;$Destination", 'User') }
} catch {
    if ($added) { [Environment]::SetEnvironmentVariable('Path', $oldPath, 'User') }
    # Keep partial files for inspection; never remove an arbitrary user destination.
    throw
}
Write-Output "Installed to $Destination; reopen the terminal for vibecut-cli on PATH."
