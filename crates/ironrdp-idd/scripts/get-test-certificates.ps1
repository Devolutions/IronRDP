[CmdletBinding()]
param(
    [string]$CertsUrl = 'https://raw.githubusercontent.com/Devolutions/psign/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode',
    [string]$CodeSignCertPassword = 'CodeSign123!'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$sha256 = [Security.Cryptography.SHA256]::Create()
try {
    $cacheKey = [BitConverter]::ToString($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($CertsUrl))).Replace('-', '').ToLowerInvariant()
}
finally {
    $sha256.Dispose()
}
$directory = Join-Path $env:LOCALAPPDATA "IronRDP\TestSigning\$cacheKey"
New-Item -ItemType Directory -Path $directory -Force | Out-Null
$caPath = Join-Path $directory 'authenticode-test-ca.crt'
$pfxPath = Join-Path $directory 'authenticode-test-cert.pfx'
$signerPath = Join-Path $directory 'authenticode-test-cert.cer'

foreach ($file in 'authenticode-test-ca.crt', 'authenticode-test-cert.pfx') {
    $path = Join-Path $directory $file
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Invoke-WebRequest -Uri "$CertsUrl/$file" -OutFile $path -ErrorAction Stop
    }
}

$signer = [Security.Cryptography.X509Certificates.X509Certificate2]::new(
    $pfxPath, $CodeSignCertPassword, [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
try {
    [IO.File]::WriteAllBytes($signerPath, $signer.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
}
finally {
    $signer.Dispose()
}

[pscustomobject]@{ RootCertificate = $caPath; SignerCertificate = $signerPath; Pfx = $pfxPath }
