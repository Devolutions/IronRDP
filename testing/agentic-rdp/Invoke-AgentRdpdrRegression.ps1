[CmdletBinding()]
param(
    # Supplying this value is the operator's authorization to contact this endpoint.
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $AuthorizedEndpoint,

    # Prefer an in-memory PSCredential over plaintext command-line credentials.
    [System.Management.Automation.PSCredential] $Credential,

    # These narrowly scoped process-local variables are the non-interactive credential path.
    [string] $EnvironmentUsername = $env:IRONRDP_RDPDR_USERNAME,

    [string] $EnvironmentPassword = $env:IRONRDP_RDPDR_PASSWORD,

    [Parameter(Mandatory)]
    [ValidatePattern('^[A-Za-z]:\\$')]
    [string] $VolumeRoot,

    [ValidatePattern('^[A-Za-z0-9_]{1,7}$')]
    [string] $DriveName = 'IRDRP',

    [string] $AgentPath = (Join-Path (Join-Path $PSScriptRoot '..\..') 'target\release\ironrdp-agent.exe'),

    [string] $ArtifactsDir = (Join-Path $env:TEMP 'ironrdp-rdpdr-regression-artifacts'),

    # Performs input, path, and credential-boundary validation without contacting the endpoint.
    [switch] $ValidateOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:RunId = [Guid]::NewGuid().ToString('N')
$script:RunName = "ironrdp-rdpdr-regression-$script:RunId"
$script:DaemonEndpoint = "\\.\pipe\ironrdp-rdpdr-regression-$PID-$script:RunId"
$script:PayloadDirectory = Join-Path $VolumeRoot $script:RunName
$script:ArtifactDirectory = $null
$script:DaemonProcess = $null
$script:Connected = $false
$script:RemoteCleanupPaths = [System.Collections.Generic.List[string]]::new()

function Get-AuthorizedCredentialValues {
    if ($null -ne $Credential) {
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($Credential.Password)
        try {
            return [pscustomobject]@{
                Username = $Credential.UserName
                Password = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
            }
        }
        finally {
            [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        }
    }

    if ([string]::IsNullOrWhiteSpace($EnvironmentUsername) -or [string]::IsNullOrWhiteSpace($EnvironmentPassword)) {
        throw 'provide -Credential or set IRONRDP_RDPDR_USERNAME and IRONRDP_RDPDR_PASSWORD in this process'
    }

    [pscustomobject]@{
        Username = $EnvironmentUsername
        Password = $EnvironmentPassword
    }
}

function Assert-RegressionConfiguration {
    if ([string]::IsNullOrWhiteSpace($AuthorizedEndpoint)) {
        throw 'an authorized endpoint is required'
    }
    if (-not (Test-Path -LiteralPath $VolumeRoot -PathType Container)) {
        throw "volume root does not exist: $VolumeRoot"
    }
    if ($VolumeRoot.Length -ne 3) {
        throw 'volume root must use the exact X:\ form'
    }
}

function Get-BoundedArtifactDirectory {
    $root = [IO.Path]::GetFullPath($ArtifactsDir)
    $path = Join-Path $root $script:RunName
    if ($path.Length -gt 240) {
        throw 'artifact path exceeds the 240-character safety limit'
    }
    return $path
}

function Invoke-Agent {
    param(
        [Parameter(ValueFromRemainingArguments)]
        [string[]] $Arguments
    )

    $output = & $AgentPath --endpoint $script:DaemonEndpoint @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        $message = ($output | Out-String).Trim()
        throw "ironrdp-agent $($Arguments -join ' ') failed: $message"
    }
    return $output
}

function Wait-AgentReady {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        try {
            Invoke-Agent status | Out-Null
            return
        }
        catch {
            Start-Sleep -Milliseconds 250
        }
    } while ([DateTime]::UtcNow -lt $deadline)

    throw 'timed out waiting for the isolated ironrdp-agent daemon'
}

function Wait-AgentConnected {
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        $status = Invoke-Agent status
        if (($status | Out-String) -match 'state:\s+Connected') {
            return
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)

    throw 'timed out waiting for the authorized RDP session to connect'
}

function Invoke-RemoteRunDialogCommand {
    param(
        [Parameter(Mandatory)]
        [string] $Command
    )

    # These are RDP input events for the remote endpoint only; they do not automate the local desktop.
    Invoke-Agent key-scancode --scancode 0xE05B --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x13 --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x13 --pressed false | Out-Null
    Invoke-Agent key-scancode --scancode 0xE05B --pressed false | Out-Null
    Start-Sleep -Milliseconds 500

    Invoke-Agent key-scancode --scancode 0x1D --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x1E --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x1E --pressed false | Out-Null
    Invoke-Agent key-scancode --scancode 0x1D --pressed false | Out-Null

    for ($offset = 0; $offset -lt $Command.Length; $offset += 48) {
        $length = [Math]::Min(48, $Command.Length - $offset)
        Invoke-Agent type-unicode --text $Command.Substring($offset, $length) | Out-Null
        Start-Sleep -Milliseconds 75
    }

    Invoke-Agent key-scancode --scancode 0x1C --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x1C --pressed false | Out-Null
}

function Write-RemoteScript {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [string] $Contents
    )

    $bytes = [Text.Encoding]::UTF8.GetBytes($Contents)
    if ($bytes.Length -gt 8192) {
        throw 'remote script exceeds the 8 KiB staging limit'
    }
    $encoded = [Convert]::ToBase64String($bytes)
    $command = @"
`$directory = Split-Path -LiteralPath '$Path' -Parent
New-Item -ItemType Directory -Path `$directory -Force | Out-Null
[IO.File]::WriteAllBytes('$Path', [Convert]::FromBase64String('$encoded'))
"@
    Invoke-Agent now powershell $command | Out-Null
}

function Remove-RemoteFiles {
    param(
        [Parameter(Mandatory)]
        [System.Collections.Generic.List[string]] $Paths
    )

    if ($Paths.Count -eq 0) {
        return
    }

    $literalPaths = ($Paths | ForEach-Object { "'$($_.Replace("'", "''"))'" }) -join ','
    Invoke-Agent now powershell "Remove-Item -LiteralPath $literalPaths -Force -ErrorAction SilentlyContinue" | Out-Null
}

function Wait-RedirectedResult {
    param(
        [Parameter(Mandatory)]
        [string] $Path
    )

    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            return (Get-Content -LiteralPath $Path -Raw).Trim()
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "timed out waiting for redirected result: $Path"
}

function Write-LogTail {
    param(
        [Parameter(Mandatory)]
        [string] $SourcePath,

        [Parameter(Mandatory)]
        [string] $DestinationPath
    )

    if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) {
        return
    }

    Get-Content -LiteralPath $SourcePath -Tail 200 |
        ForEach-Object {
            if ($_.Length -gt 4096) {
                $_.Substring(0, 4096)
            }
            else {
                $_
            }
        } |
        Set-Content -LiteralPath $DestinationPath -Encoding utf8NoBOM
    Remove-Item -LiteralPath $SourcePath -Force
}

Assert-RegressionConfiguration
$credentialValues = Get-AuthorizedCredentialValues
try {
    if ($ValidateOnly) {
        [pscustomobject]@{
            ValidateOnly = $true
            AuthorizedEndpoint = $AuthorizedEndpoint
            DriveName = $DriveName
            VolumeRoot = $VolumeRoot
        }
        return
    }

    if (-not (Test-Path -LiteralPath $AgentPath -PathType Leaf)) {
        throw "ironrdp-agent executable was not found: $AgentPath"
    }

    $script:ArtifactDirectory = Get-BoundedArtifactDirectory
    New-Item -ItemType Directory -Path $script:ArtifactDirectory -Force | Out-Null
    New-Item -ItemType Directory -Path $script:PayloadDirectory -ErrorAction Stop | Out-Null

    $payloadPath = Join-Path $script:PayloadDirectory 'payload.bin'
    $payloadBytes = [byte[]]::new(65536)
    for ($index = 0; $index -lt $payloadBytes.Length; $index++) {
        $payloadBytes[$index] = $index % 251
    }
    [IO.File]::WriteAllBytes($payloadPath, $payloadBytes)
    $expectedHash = (Get-FileHash -LiteralPath $payloadPath -Algorithm SHA256).Hash

    $previousLogLevel = $env:IRONRDP_LOG
    try {
        $env:IRONRDP_LOG = 'warn'
        $daemonArguments = @(
            '--endpoint', $script:DaemonEndpoint,
            'daemon-start',
            '--rdpdr-drive', "$DriveName=$VolumeRoot"
        )
        $script:DaemonProcess = Start-Process `
            -FilePath $AgentPath `
            -ArgumentList $daemonArguments `
            -RedirectStandardOutput (Join-Path $script:ArtifactDirectory 'daemon.stdout.log') `
            -RedirectStandardError (Join-Path $script:ArtifactDirectory 'daemon.stderr.log') `
            -PassThru
    }
    finally {
        $env:IRONRDP_LOG = $previousLogLevel
    }

    Wait-AgentReady

    $previousUsername = $env:RDP_USERNAME
    $previousPassword = $env:RDP_PASSWORD
    try {
        $env:RDP_USERNAME = $credentialValues.Username
        $env:RDP_PASSWORD = $credentialValues.Password
        Invoke-Agent connect --server $AuthorizedEndpoint | Out-Null
    }
    finally {
        $env:RDP_USERNAME = $previousUsername
        $env:RDP_PASSWORD = $previousPassword
    }
    $script:Connected = $true
    Wait-AgentConnected

    $remoteDirectory = "\\tsclient\$DriveName\$script:RunName"
    $remotePayloadPath = Join-Path $remoteDirectory 'payload.bin'
    $directResultPath = Join-Path $script:PayloadDirectory 'direct.sha256'
    $explorerResultPath = Join-Path $script:PayloadDirectory 'explorer.sha256'
    $remoteScriptDirectory = 'C:\Users\Public\Documents'
    $directScriptPath = Join-Path $remoteScriptDirectory "ironrdp-rdpdr-direct-$script:RunId.ps1"
    $explorerScriptPath = Join-Path $remoteScriptDirectory "ironrdp-rdpdr-explorer-$script:RunId.ps1"
    $directDestination = Join-Path $remoteScriptDirectory "ironrdp-rdpdr-direct-$script:RunId.bin"
    $explorerDestination = Join-Path $remoteScriptDirectory "ironrdp-rdpdr-explorer-$script:RunId.bin"
    foreach ($remoteCleanupPath in @(
            $directScriptPath,
            $explorerScriptPath,
            $directDestination,
            $explorerDestination
        )) {
        [void] $script:RemoteCleanupPaths.Add($remoteCleanupPath)
    }

    $directScript = @'
$source = '__SOURCE__'
$destination = '__DESTINATION__'
$result = '__RESULT__'
Copy-Item -LiteralPath $source -Destination $destination -Force
$hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
[IO.File]::WriteAllText($result, $hash, [Text.UTF8Encoding]::new($false))
Remove-Item -LiteralPath $destination -Force
Remove-Item -LiteralPath $PSCommandPath -Force
'@
    $directScript = $directScript.Replace('__SOURCE__', $remotePayloadPath).Replace('__DESTINATION__', $directDestination).Replace('__RESULT__', (Join-Path $remoteDirectory 'direct.sha256'))
    Write-RemoteScript $directScriptPath $directScript
    Invoke-RemoteRunDialogCommand "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -File $directScriptPath"
    $directHash = Wait-RedirectedResult $directResultPath
    if ($directHash -ne $expectedHash) {
        throw 'direct PowerShell copy SHA-256 did not match the deterministic payload'
    }

    $explorerScript = @'
$source = '__SOURCE__'
$destination = '__DESTINATION__'
$result = '__RESULT__'
$expectedLength = __LENGTH__
$shell = New-Object -ComObject Shell.Application
$folder = $shell.NameSpace((Split-Path -LiteralPath $destination -Parent))
if ($null -eq $folder) { throw 'Explorer destination folder is unavailable' }
$folder.CopyHere($source, 16)
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while ((-not (Test-Path -LiteralPath $destination -PathType Leaf)) -or ((Get-Item -LiteralPath $destination).Length -ne $expectedLength)) {
    if ([DateTime]::UtcNow -ge $deadline) { throw 'Explorer copy did not complete' }
    Start-Sleep -Milliseconds 200
}
$hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
[IO.File]::WriteAllText($result, $hash, [Text.UTF8Encoding]::new($false))
Remove-Item -LiteralPath $destination -Force
Remove-Item -LiteralPath $PSCommandPath -Force
'@
    $explorerScript = $explorerScript.Replace('__SOURCE__', $remotePayloadPath).Replace('__DESTINATION__', $explorerDestination).Replace('__RESULT__', (Join-Path $remoteDirectory 'explorer.sha256')).Replace('__LENGTH__', $payloadBytes.Length)
    Write-RemoteScript $explorerScriptPath $explorerScript
    Invoke-RemoteRunDialogCommand "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -File $explorerScriptPath"
    $explorerHash = Wait-RedirectedResult $explorerResultPath
    if ($explorerHash -ne $expectedHash) {
        throw 'Explorer-shell copy SHA-256 did not match the deterministic payload'
    }

    [pscustomobject]@{
        AuthorizedEndpoint = $AuthorizedEndpoint
        DriveName = $DriveName
        ExpectedSha256 = $expectedHash
        DirectPowerShellSha256 = $directHash
        ExplorerShellSha256 = $explorerHash
        Artifacts = $script:ArtifactDirectory
    } | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $script:ArtifactDirectory 'result.json') -Encoding utf8NoBOM
    Get-Content -LiteralPath (Join-Path $script:ArtifactDirectory 'result.json') -Raw
}
finally {
    if ($script:Connected -and $script:RemoteCleanupPaths.Count -gt 0) {
        try {
            Remove-RemoteFiles $script:RemoteCleanupPaths
        }
        catch {
            Write-Warning "Could not remove remote temporary files: $($_.Exception.Message)"
        }
    }
    if ($null -ne $script:DaemonProcess -and -not $script:DaemonProcess.HasExited) {
        Stop-Process -Id $script:DaemonProcess.Id -Force
        $script:DaemonProcess.WaitForExit()
    }
    if ($null -ne $script:ArtifactDirectory) {
        Write-LogTail (Join-Path $script:ArtifactDirectory 'daemon.stdout.log') (Join-Path $script:ArtifactDirectory 'daemon.stdout.tail.log')
        Write-LogTail (Join-Path $script:ArtifactDirectory 'daemon.stderr.log') (Join-Path $script:ArtifactDirectory 'daemon.stderr.tail.log')
    }
    if (Test-Path -LiteralPath $script:PayloadDirectory -PathType Container) {
        Remove-Item -LiteralPath $script:PayloadDirectory -Recurse -Force
    }
    $credentialValues.Password = $null
}
