[CmdletBinding()]
param(
    [Parameter()]
    [string]$DriverPath = "target\release\ironrdp_idd.dll",

    [Parameter()]
    [string]$InfPath = "crates\ironrdp-idd\IronRdpIdd.inf",

    [Parameter()]
    [string]$CertSubject = "CN=Test Code Signing Certificate",

    [Parameter()]
    [string]$CertsUrl = 'https://raw.githubusercontent.com/Devolutions/psign/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode',

    [Parameter()]
    [string]$CodeSignCertPassword = 'CodeSign123!'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-WindowsKitTool {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ExeName,

        [Parameter()]
        [string]$WindowsKitsRoot = $(if ($env:IRONRDP_WINDOWS_KITS_ROOT) { $env:IRONRDP_WINDOWS_KITS_ROOT } else { 'C:\Program Files (x86)\Windows Kits\10' }),

        [Parameter()]
        [string]$Arch = "x64"
    )

    $cmd = Get-Command $ExeName -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $binRoot = Join-Path $WindowsKitsRoot 'bin'
    if (-not (Test-Path $binRoot)) {
        return $null
    }

    $candidates = @(Get-ChildItem $binRoot -Directory -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName (Join-Path $Arch $ExeName) } |
            Where-Object { Test-Path $_ })

    if ($candidates.Count -eq 0) {
        # Fallback: recursive search (slower, but helpful on odd layouts)
        $candidates = @(Get-ChildItem $binRoot -Recurse -Filter $ExeName -ErrorAction SilentlyContinue |
                Select-Object -ExpandProperty FullName)
    }

    if ($candidates.Count -eq 0) {
        return $null
    }

    # Prefer the highest version folder if multiple are present.
    return @($candidates | Sort-Object)[-1]
}

$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path

$driverFullPath = (Resolve-Path (Join-Path $workspaceRoot $DriverPath)).Path
$infFullPath = (Resolve-Path (Join-Path $workspaceRoot $InfPath)).Path

$infDir = Split-Path -Parent $infFullPath
$stagedDriverPath = Join-Path $infDir 'IronRdpIdd.dll'

Copy-Item -Path $driverFullPath -Destination $stagedDriverPath -Force

$psign = (Get-Command psign-tool -ErrorAction Stop).Source
$inf2cat = Resolve-WindowsKitTool -ExeName 'Inf2Cat.exe'
if (-not $inf2cat) {
    throw "Inf2Cat.exe not found (install WDK)"
}

$catPath = Join-Path $infDir 'IronRdpIdd.cat'
$certificates = & (Join-Path $PSScriptRoot 'get-test-certificates.ps1') -CertsUrl $CertsUrl -CodeSignCertPassword $CodeSignCertPassword
$signer = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certificates.SignerCertificate)
try {
    if ($signer.Subject -ne $CertSubject) {
        throw "Unexpected signing certificate subject: $($signer.Subject)"
    }
    $thumbprint = $signer.Thumbprint
}
finally {
    $signer.Dispose()
}
$storePath = "cert:\CurrentUser\My\$thumbprint"
$alreadyPresent = Test-Path -LiteralPath $storePath
try {
    # psign 0.8.0 requires a persisted key provider; My does not establish certificate trust.
    if (-not $alreadyPresent) {
        Import-PfxCertificate -FilePath $certificates.Pfx -Password (ConvertTo-SecureString $CodeSignCertPassword -AsPlainText -Force) -CertStoreLocation cert:\CurrentUser\My | Out-Null
    }
    if (-not (Get-Item -LiteralPath $storePath).HasPrivateKey) {
        throw "Signing certificate has no private key: $storePath"
    }
    & $psign --mode windows sign --cert-sha1 $thumbprint --digest sha256 $stagedDriverPath
    if ($LASTEXITCODE -ne 0) { throw "psign failed for driver DLL (exit $LASTEXITCODE)" }

    & $inf2cat /driver:$infDir /os:10_X64
    if ($LASTEXITCODE -ne 0) { throw "Inf2Cat failed (exit $LASTEXITCODE)" }
    if (-not (Test-Path -LiteralPath $catPath)) { throw "Catalog not found after Inf2Cat: $catPath" }

    & $psign --mode windows sign --cert-sha1 $thumbprint --digest sha256 $catPath
    if ($LASTEXITCODE -ne 0) { throw "psign failed for catalog (exit $LASTEXITCODE)" }

    & $psign portable trust-verify-pe $stagedDriverPath --trusted-ca $certificates.RootCertificate --revocation-mode off
    if ($LASTEXITCODE -ne 0) { throw "Driver signature verification failed (exit $LASTEXITCODE)" }
    & $psign portable trust-verify-catalog $catPath --trusted-ca $certificates.RootCertificate --revocation-mode off
    if ($LASTEXITCODE -ne 0) { throw "Catalog signature verification failed (exit $LASTEXITCODE)" }
    & $psign portable verify-catalog-member $stagedDriverPath --catalog $catPath
    if ($LASTEXITCODE -ne 0) { throw "Catalog membership verification failed (exit $LASTEXITCODE)" }
}
finally {
    if (-not $alreadyPresent -and (Test-Path -LiteralPath $storePath)) {
        Remove-Item -LiteralPath $storePath -DeleteKey -ErrorAction Stop
    }
}

Write-Host "Staged: $stagedDriverPath"
Write-Host "Signed: $stagedDriverPath"
Write-Host "Signed: $catPath"
