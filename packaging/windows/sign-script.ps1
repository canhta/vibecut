param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Thumbprint, [Parameter(Mandatory)][string]$Timestamp)
$ErrorActionPreference = 'Stop'
$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$Thumbprint"
$result = Set-AuthenticodeSignature -LiteralPath $Path -Certificate $certificate -HashAlgorithm SHA256 -TimestampServer $Timestamp
if ($result.Status -ne 'Valid') { throw "Signing failed: $($result.Status)" }
