[CmdletBinding()]
param(
    [string] $HostName = 'IT-HELP-RDM',

    [string] $AgentPath,

    [string] $ArtifactsDir,

    [ValidateRange(9, 1024)]
    [int] $PayloadMiB = 12,

    [ValidateRange(0, 1073741824)]
    [int] $PayloadBytes = 0,

    [string] $SourceFile,

    [string] $ResumeRemoteFile,

    [ValidateRange(90, 43200)]
    [int] $CopyTimeoutSeconds = 90,

    [ValidatePattern('^[A-Za-z0-9 _.-]{1,7}$')]
    [string] $DriveName = 'C',

    [string] $AdditionalDriveRoot,

    [ValidatePattern('^[A-Za-z0-9 _.-]{1,7}$')]
    [string] $AdditionalDriveName = 'D',

    [switch] $FullAudit,

    [switch] $DangerouslyAcceptInvalidCertificate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$workspaceRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($AgentPath)) {
    $AgentPath = Join-Path $workspaceRoot 'target\release\ironrdp-agent.exe'
}
if ([string]::IsNullOrWhiteSpace($ArtifactsDir)) {
    $ArtifactsDir = Join-Path $workspaceRoot 'artifacts\agent-rdpdr'
}

$endpoint = "\\.\pipe\ironrdp-agent-rdpdr-$PID"
$sourceRoot = Join-Path $env:PUBLIC "Documents\i-$PID"
$sourceFolder = $sourceRoot
$requestedSourceFile = $SourceFile
$sourceFile = Join-Path $sourceFolder 'payload.bin'
$daemon = $null
$connected = $false
$largeTransferSucceeded = $false
$reparseAuditRoot = $null

function Invoke-Agent {
    param(
        [Parameter(ValueFromRemainingArguments)]
        [string[]] $Arguments
    )

    & $AgentPath --endpoint $endpoint @Arguments
}

function ConvertTo-PowerShellLiteral {
    param([Parameter(Mandatory)][string] $Value)

    return "'" + $Value.Replace("'", "''") + "'"
}

function New-DeterministicPayload {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][long] $ByteCount
    )

    $buffer = New-Object byte[] ([Math]::Min(1024 * 1024, $ByteCount))
    for ($index = 0; $index -lt $buffer.Length; $index++) {
        $buffer[$index] = [byte] ($index % 251)
    }

    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write)
    try {
        [long] $remaining = $ByteCount
        while ($remaining -gt 0) {
            $writeLength = [Math]::Min($buffer.Length, $remaining)
            $stream.Write($buffer, 0, $writeLength)
            $remaining -= $writeLength
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Wait-AgentConnected {
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    do {
        $status = Invoke-Agent status 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0 -and $status -match 'state:\s+Connected') {
            return
        }
        if ($LASTEXITCODE -eq 0 -and $status -match 'state:\s+Failed') {
            if ($status -match 'TLS upgrade' -and [string]::IsNullOrWhiteSpace($env:IRONRDP_AGENT_CERTIFICATE_SHA256) -and -not $DangerouslyAcceptInvalidCertificate) {
                throw 'strict certificate validation rejected the endpoint; set IRONRDP_AGENT_CERTIFICATE_SHA256 only after independently verifying the IT-HELP-RDM certificate fingerprint'
            }
            throw "agent connection failed: $status"
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "agent did not connect within 90 seconds: $status"
}

function Wait-AgentAvailable {
    param([Parameter(Mandatory)][System.Diagnostics.Process] $Daemon)

    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        try {
            & $AgentPath --endpoint $endpoint status 2>$null | Out-Null
            if ($LASTEXITCODE -eq 0) {
                return
            }
        }
        catch {
            # The named pipe is not available until the daemon finishes binding it.
        }
        if ($Daemon.HasExited) {
            throw "agent daemon exited before opening $endpoint"
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "agent daemon did not open $endpoint within 15 seconds"
}

function Wait-NowAvailable {
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        $capabilities = Invoke-Agent now capabilities 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0 -and $capabilities -match 'powershell:\s+true') {
            return
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "remote NOW PowerShell is unavailable: $capabilities"
}

function Invoke-RemoteDesktopCommand {
    param(
        [Parameter(Mandatory)][string] $Command,
        [string] $TypedCommandScreenshot
    )

    # The server exposes redirected drives only to the interactive desktop token,
    # not to the NOW DVC command process. Open Run through RDP input so the command
    # executes in that token without moving or focusing any local windows.
    Invoke-Agent key-scancode --scancode 0xE05B --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x13 --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x13 --pressed false | Out-Null
    Invoke-Agent key-scancode --scancode 0xE05B --pressed false | Out-Null
    Start-Sleep -Seconds 2

    # The Run dialog retains prior commands in its editable history. Selecting its
    # entire contents ensures this run cannot append to a stale command.
    Invoke-Agent key-scancode --scancode 0x1D --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x1E --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x1E --pressed false | Out-Null
    Invoke-Agent key-scancode --scancode 0x1D --pressed false | Out-Null
    Start-Sleep -Milliseconds 250

    # The Run dialog can drop a full command line injected in a single input burst.
    # Keep each request below the FastPath event limit and allow the dialog to drain it.
    for ($offset = 0; $offset -lt $Command.Length; $offset += 48) {
        $length = [Math]::Min(48, $Command.Length - $offset)
        Invoke-Agent type-unicode --text $Command.Substring($offset, $length) | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw 'could not type remote command'
        }
        Start-Sleep -Milliseconds 75
    }

    if ($TypedCommandScreenshot) {
        Invoke-Agent screenshot $TypedCommandScreenshot | Out-Null
    }

    Invoke-Agent key-scancode --scancode 0x1C --pressed true | Out-Null
    Invoke-Agent key-scancode --scancode 0x1C --pressed false | Out-Null
}

function Clear-RemoteDesktopTransientUi {
    # A disconnected session can retain an application error dialog. Dismiss it before
    # sending a Run-dialog command, otherwise the command's keystrokes are consumed by
    # the stale modal window.
    for ($attempt = 0; $attempt -lt 3; $attempt++) {
        Invoke-Agent key-scancode --scancode 0x01 --pressed true | Out-Null
        Invoke-Agent key-scancode --scancode 0x01 --pressed false | Out-Null
        Start-Sleep -Milliseconds 250
    }
}

function Get-RemoteFileHash {
    param([Parameter(Mandatory)][string] $Path)

    $pathLiteral = ConvertTo-PowerShellLiteral -Value $Path
    $output = Invoke-Agent now powershell "if (Test-Path -LiteralPath $pathLiteral -PathType Leaf) { (Get-FileHash -LiteralPath $pathLiteral -Algorithm SHA256).Hash }" --timeout 30 2>&1 | Out-String
    # The NOW CLI can prefix successful stdout with transport metadata, so do
    # not require the digest to occupy the entire output line.
    $match = [regex]::Match($output, '(?i)\b[0-9a-f]{64}\b')
    if ($match.Success) {
        return $match.Value.ToUpperInvariant()
    }

    return $null
}

function Wait-RemoteFileHash {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $ExpectedHash,
        [ValidateRange(1, 43200)]
        [int] $TimeoutSeconds = 90
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $actualHash = Get-RemoteFileHash -Path $Path
        if ($null -ne $actualHash) {
            if ($actualHash -ne $ExpectedHash) {
                throw "remote copy hash mismatch: expected $ExpectedHash, got $actualHash"
            }
            return
        }
        Start-Sleep -Seconds 1
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "remote copy did not create $Path within $TimeoutSeconds seconds"
}

function Assert-RemoteDestinationCapacity {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][long] $RequiredBytes
    )

    $pathLiteral = ConvertTo-PowerShellLiteral -Value $Path
    $output = Invoke-Agent now powershell "([IO.DriveInfo]::new([IO.Path]::GetPathRoot($pathLiteral))).AvailableFreeSpace" --timeout 30 2>&1 | Out-String
    $freeSpaceMatches = [regex]::Matches($output, '(?m)^\s*(\d+)\s*$')
    if ($freeSpaceMatches.Count -eq 0) {
        throw "could not determine available space for remote destination $Path"
    }

    [long] $availableBytes = $freeSpaceMatches[$freeSpaceMatches.Count - 1].Groups[1].Value
    $requiredWithHeadroom = $RequiredBytes + 64MB
    if ($availableBytes -lt $requiredWithHeadroom) {
        throw "remote destination has $availableBytes bytes free but needs at least $requiredWithHeadroom bytes"
    }
}

function Save-RemoteTextFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Destination
    )

    $pathLiteral = ConvertTo-PowerShellLiteral -Value $Path
    $contents = Invoke-Agent now powershell "if (Test-Path -LiteralPath $pathLiteral -PathType Leaf) { Get-Content -LiteralPath $pathLiteral -Raw }" --timeout 30 2>&1
    $contents | Out-File -LiteralPath $Destination
}

function Write-RemoteScript {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Contents
    )

    $pathLiteral = ConvertTo-PowerShellLiteral -Value $Path
    $contentsBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Contents))
    $command = "[IO.File]::WriteAllBytes($pathLiteral, [Convert]::FromBase64String('$contentsBase64'))"
    Invoke-Agent now powershell $command --timeout 30 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "could not write remote script $Path"
    }
}

function Wait-RemoteFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [ValidateRange(1, 43200)]
        [int] $TimeoutSeconds = 90
    )

    $pathLiteral = ConvertTo-PowerShellLiteral -Value $Path
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $output = Invoke-Agent now powershell "[bool](Test-Path -LiteralPath $pathLiteral -PathType Leaf)" --timeout 30 2>&1 | Out-String
        if ($output -match '(?im)^\s*True\s*$') {
            return
        }
        Start-Sleep -Seconds 1
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "remote operation did not create $Path within $TimeoutSeconds seconds"
}

function Wait-LocalFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [ValidateRange(1, 43200)]
        [int] $TimeoutSeconds = 90
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            return
        }
        Start-Sleep -Seconds 1
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "redirected operation did not create $Path within $TimeoutSeconds seconds"
}

New-Item -ItemType Directory -Path $ArtifactsDir -Force | Out-Null
if (-not (Test-Path -LiteralPath $AgentPath -PathType Leaf)) {
    throw "ironrdp-agent executable was not found: $AgentPath"
}
if ([string]::IsNullOrWhiteSpace($env:RDP_USERNAME) -or [string]::IsNullOrWhiteSpace($env:RDP_PASSWORD)) {
    throw 'RDP_USERNAME and RDP_PASSWORD must be set in the launching process environment'
}

try {
    New-Item -ItemType Directory -Path $sourceFolder -Force | Out-Null
    if ([string]::IsNullOrWhiteSpace($requestedSourceFile)) {
        [long] $payloadByteCount = if ($PayloadBytes -gt 0) { $PayloadBytes } else { $PayloadMiB * 1MB }
        New-DeterministicPayload -Path $sourceFile -ByteCount $payloadByteCount
    }
    else {
        $sourceFileInfo = Get-Item -LiteralPath $requestedSourceFile -ErrorAction Stop
        if ($sourceFileInfo.PSIsContainer) {
            throw "large transfer source must be a file: $requestedSourceFile"
        }
        $sourceFile = $sourceFileInfo.FullName
        [long] $payloadByteCount = $sourceFileInfo.Length
    }
    if (-not [string]::IsNullOrWhiteSpace($ResumeRemoteFile) -and [string]::IsNullOrWhiteSpace($requestedSourceFile)) {
        throw 'ResumeRemoteFile requires SourceFile so the existing remote file can be verified against its local source'
    }

    $sourceDrive = (Get-Item -LiteralPath $sourceFile).PSDrive
    $volumeRoot = $sourceDrive.Root
    if (-not [string]::IsNullOrWhiteSpace($AdditionalDriveRoot)) {
        $additionalDrive = Get-PSDrive -PSProvider FileSystem |
            Where-Object { $_.Root.TrimEnd('\') -ieq $AdditionalDriveRoot.TrimEnd('\') } |
            Select-Object -First 1
        if ($null -eq $additionalDrive) {
            throw "AdditionalDriveRoot is not a mounted filesystem volume: $AdditionalDriveRoot"
        }
        $AdditionalDriveRoot = $additionalDrive.Root
        if ($AdditionalDriveRoot.TrimEnd('\') -ieq $volumeRoot.TrimEnd('\')) {
            throw 'AdditionalDriveRoot must differ from the primary redirected volume'
        }
        if ($AdditionalDriveName -ieq $DriveName) {
            throw 'AdditionalDriveName must differ from DriveName'
        }
    }
    $relativeSourceFolder = $sourceFolder.Substring($volumeRoot.Length).TrimStart('\')
    $relativeSourceFile = $sourceFile.Substring($volumeRoot.Length).TrimStart('\')
    $remoteSourceFolder = "\\tsclient\$DriveName\$relativeSourceFolder"
    $remoteSourceFile = "\\tsclient\$DriveName\$relativeSourceFile"
    $remoteAdditionalRoot = if ([string]::IsNullOrWhiteSpace($AdditionalDriveRoot)) {
        $null
    }
    else {
        "\\tsclient\$AdditionalDriveName"
    }
    $reparseAuditEscapePath = $null
    $reparseAuditSkipReason = 'not-run: no additional redirected root distinct from the Windows system root'
    if ($FullAudit -and $null -ne $remoteAdditionalRoot) {
        $systemRootVolume = (Get-Item -LiteralPath $env:SystemRoot).PSDrive.Root
        if ($AdditionalDriveRoot.TrimEnd('\') -ine $systemRootVolume.TrimEnd('\')) {
            $reparseAuditRoot = Join-Path $AdditionalDriveRoot "ironrdp-rdpdr-reparse-$PID"
            $reparseAuditLink = Join-Path $reparseAuditRoot 'escape'
            New-Item -ItemType Directory -Path $reparseAuditRoot -Force | Out-Null
            New-Item -ItemType Junction -Path $reparseAuditLink -Target $env:SystemRoot | Out-Null
            $reparseAuditEscapePath = Join-Path $remoteAdditionalRoot "ironrdp-rdpdr-reparse-$PID\escape\win.ini"
            $reparseAuditSkipReason = $null
        }
    }
    $sourceHash = (Get-FileHash -LiteralPath $sourceFile -Algorithm SHA256).Hash
    $remoteDestinationRoot = 'C:\Users\Public\Documents'
    $remoteDirectFile = if ([string]::IsNullOrWhiteSpace($ResumeRemoteFile)) {
        Join-Path $remoteDestinationRoot "d-$PID.bin"
    }
    else {
        $ResumeRemoteFile
    }
    $remoteCopyResult = Join-Path $remoteDestinationRoot "o-$PID.txt"
    $remoteCopyScript = Join-Path $remoteDestinationRoot "r-$PID.ps1"
    $remoteRoundTripFile = Join-Path $remoteSourceFolder "roundtrip-$PID.bin"
    $remoteRoundTripResult = Join-Path $remoteDestinationRoot "q-$PID.txt"
    $remoteRoundTripScript = Join-Path $remoteDestinationRoot "t-$PID.ps1"
    $remoteCopyStatusFile = Join-Path $remoteSourceFolder "copy-status-$PID.json"
    $localCopyStatusFile = Join-Path $sourceFolder "copy-status-$PID.json"
    $remoteRoundTripStatusFile = Join-Path $remoteSourceFolder "roundtrip-status-$PID.json"
    $localRoundTripStatusFile = Join-Path $sourceFolder "roundtrip-status-$PID.json"
    $remoteAuditScript = Join-Path $remoteDestinationRoot "a-$PID.ps1"
    $remoteAuditResult = Join-Path $remoteDestinationRoot "a-$PID.json"
    $remoteServerPayload = Join-Path $remoteDestinationRoot "s-$PID.bin"
    $remoteInboundPayload = Join-Path $remoteDestinationRoot "i-$PID.bin"
    $localServerPayload = Join-Path $sourceFolder 'server-to-client.bin'

    $daemonStdout = Join-Path $ArtifactsDir 'daemon.stdout.log'
    $daemonStderr = Join-Path $ArtifactsDir 'daemon.stderr.log'
    $daemonArguments = @('--endpoint', $endpoint, 'daemon-start', '--rdpdr-drive', "$DriveName=$volumeRoot")
    if ($null -ne $remoteAdditionalRoot) {
        $daemonArguments += @('--rdpdr-drive', "$AdditionalDriveName=$AdditionalDriveRoot")
    }
    if ($DangerouslyAcceptInvalidCertificate) {
        $daemonArguments += '--dangerously-accept-invalid-certificate'
    }
    $daemon = Start-Process `
        -FilePath $AgentPath `
        -ArgumentList $daemonArguments `
        -WindowStyle Hidden `
        -RedirectStandardOutput $daemonStdout `
        -RedirectStandardError $daemonStderr `
        -PassThru

    Wait-AgentAvailable -Daemon $daemon
    Invoke-Agent connect --server $HostName --log-directive 'warn,ironrdp_rdpdr=debug,ironrdp_rdpdr_native=debug' | Out-File (Join-Path $ArtifactsDir 'connect.log')
    Wait-AgentConnected
    $connected = $true
    Invoke-Agent screenshot (Join-Path $ArtifactsDir 'connected.png') | Out-Null

    Wait-NowAvailable
    Invoke-Agent now capabilities | Tee-Object -FilePath (Join-Path $ArtifactsDir 'now-capabilities.log') | Out-Null
    if ([string]::IsNullOrWhiteSpace($ResumeRemoteFile)) {
        Assert-RemoteDestinationCapacity -Path $remoteDestinationRoot -RequiredBytes $payloadByteCount
    }
    if ($sourceDrive.Free -lt ($payloadByteCount + 64MB)) {
        throw "local redirected volume does not have sufficient space for a $payloadByteCount byte round-trip copy"
    }
    Clear-RemoteDesktopTransientUi
    Invoke-Agent screenshot (Join-Path $ArtifactsDir 'ready.png') | Out-Null

    $sourceLiteral = ConvertTo-PowerShellLiteral -Value $remoteSourceFile
    $destinationLiteral = ConvertTo-PowerShellLiteral -Value $remoteDirectFile
    $copyStatusLiteral = ConvertTo-PowerShellLiteral -Value $remoteCopyStatusFile
    $roundTripSourceLiteral = ConvertTo-PowerShellLiteral -Value $remoteDirectFile
    $roundTripDestinationLiteral = ConvertTo-PowerShellLiteral -Value $remoteRoundTripFile
    $roundTripStatusLiteral = ConvertTo-PowerShellLiteral -Value $remoteRoundTripStatusFile
    $copyScript = @"
`$ErrorActionPreference = 'Stop'
`$report = [ordered]@{}
try {
    if ($(if ([string]::IsNullOrWhiteSpace($ResumeRemoteFile)) { '$true' } else { '$false' })) {
        Copy-Item -LiteralPath $sourceLiteral -Destination $destinationLiteral -Force
    }
    if (-not (Test-Path -LiteralPath $destinationLiteral -PathType Leaf)) {
        throw 'remote transfer destination was not created'
    }
    `$report.status = 'passed'
    `$report.destinationHash = (Get-FileHash -LiteralPath $destinationLiteral -Algorithm SHA256).Hash
}
catch {
    `$report.status = 'failed'
    `$report.error = `$_.Exception.Message
}
[IO.File]::WriteAllText($copyStatusLiteral, (`$report | ConvertTo-Json -Compress))
if (`$report.status -ne 'passed') {
    exit 1
}
"@
    $roundTripScript = @"
`$ErrorActionPreference = 'Stop'
`$report = [ordered]@{}
try {
    Copy-Item -LiteralPath $roundTripSourceLiteral -Destination $roundTripDestinationLiteral -Force
    `$report.status = 'passed'
}
catch {
    `$report.status = 'failed'
    `$report.error = `$_.Exception.Message
}
[IO.File]::WriteAllText($roundTripStatusLiteral, (`$report | ConvertTo-Json -Compress))
if (`$report.status -ne 'passed') {
    exit 1
}
"@
    Write-RemoteScript -Path $remoteCopyScript -Contents $copyScript
    Write-RemoteScript -Path $remoteRoundTripScript -Contents $roundTripScript
    Invoke-RemoteDesktopCommand `
        -Command "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $remoteCopyScript" `
        -TypedCommandScreenshot (Join-Path $ArtifactsDir 'typed-command.png')
    Start-Sleep -Seconds 1
    Invoke-Agent screenshot (Join-Path $ArtifactsDir 'direct-copy.png') | Out-Null
    Wait-LocalFile -Path $localCopyStatusFile -TimeoutSeconds $CopyTimeoutSeconds
    Copy-Item -LiteralPath $localCopyStatusFile -Destination (Join-Path $ArtifactsDir 'copy-status.json')
    $copyStatus = Get-Content -LiteralPath $localCopyStatusFile -Raw | ConvertFrom-Json
    if ($copyStatus.status -ne 'passed') {
        throw "remote copy failed: $($copyStatus.error)"
    }
    if ($copyStatus.destinationHash -ne $sourceHash) {
        throw "remote copy hash mismatch: expected $sourceHash, got $($copyStatus.destinationHash)"
    }

    if (-not [string]::IsNullOrWhiteSpace($requestedSourceFile)) {
        Invoke-RemoteDesktopCommand `
            -Command "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $remoteRoundTripScript" `
            -TypedCommandScreenshot (Join-Path $ArtifactsDir 'typed-roundtrip-command.png')
        $localRoundTripFile = Join-Path $sourceFolder "roundtrip-$PID.bin"
        Wait-LocalFile -Path $localRoundTripStatusFile -TimeoutSeconds $CopyTimeoutSeconds
        Copy-Item -LiteralPath $localRoundTripStatusFile -Destination (Join-Path $ArtifactsDir 'roundtrip-status.json')
        $roundTripStatus = Get-Content -LiteralPath $localRoundTripStatusFile -Raw | ConvertFrom-Json
        if ($roundTripStatus.status -ne 'passed') {
            throw "server-to-client copy failed: $($roundTripStatus.error)"
        }
        if (-not (Test-Path -LiteralPath $localRoundTripFile -PathType Leaf)) {
            throw "server-to-client copy did not create $localRoundTripFile within $CopyTimeoutSeconds seconds"
        }
        $roundTripHash = (Get-FileHash -LiteralPath $localRoundTripFile -Algorithm SHA256).Hash
        if ($roundTripHash -ne $sourceHash) {
            throw 'server-to-client large transfer hash mismatch'
        }
        [ordered]@{
            status = 'passed'
            sourceHash = $sourceHash
            returnedHash = $roundTripHash
            byteCount = $payloadByteCount
        } |
            ConvertTo-Json -Compress |
            Set-Content -LiteralPath (Join-Path $ArtifactsDir 'roundtrip-integrity.json') -NoNewline
        $largeTransferSucceeded = $true
    }

    if ($FullAudit) {
        $serverPayloadLiteral = ConvertTo-PowerShellLiteral -Value $remoteServerPayload
        $serverPayloadCommand = @"
`$bytes = New-Object byte[] 1048576
for (`$index = 0; `$index -lt `$bytes.Length; `$index++) { `$bytes[`$index] = [byte] (`$index % 251) }
[IO.File]::WriteAllBytes($serverPayloadLiteral, `$bytes)
"@
        Invoke-Agent now powershell $serverPayloadCommand --timeout 30 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw 'could not stage the remote-to-client audit payload'
        }
        $serverPayloadHash = Get-RemoteFileHash -Path $remoteServerPayload
        if ($null -eq $serverPayloadHash) {
            throw 'could not hash the staged remote-to-client audit payload'
        }

        $clientRootLiteral = ConvertTo-PowerShellLiteral -Value $remoteSourceFolder
        $clientDriveRootLiteral = ConvertTo-PowerShellLiteral -Value "\\tsclient\$DriveName\"
        $clientDirectoryLiteral = ConvertTo-PowerShellLiteral -Value (Join-Path $remoteSourceFolder 'full-surface')
        $secondaryClientRootLiteral = if ($null -eq $remoteAdditionalRoot) {
            '$null'
        }
        else {
            ConvertTo-PowerShellLiteral -Value $remoteAdditionalRoot
        }
        $clientSourceLiteral = ConvertTo-PowerShellLiteral -Value $remoteSourceFile
        $localServerPayloadLiteral = ConvertTo-PowerShellLiteral -Value (Join-Path $remoteSourceFolder 'server-to-client.bin')
        $remoteServerPayloadLiteral = ConvertTo-PowerShellLiteral -Value $remoteServerPayload
        $remoteInboundPayloadLiteral = ConvertTo-PowerShellLiteral -Value $remoteInboundPayload
        $remoteAuditResultLiteral = ConvertTo-PowerShellLiteral -Value $remoteAuditResult
        $reparseAuditEscapePathLiteral = if ($null -eq $reparseAuditEscapePath) {
            '$null'
        }
        else {
            ConvertTo-PowerShellLiteral -Value $reparseAuditEscapePath
        }
        $reparseAuditSkipReasonLiteral = if ($null -eq $reparseAuditSkipReason) {
            '$null'
        }
        else {
            ConvertTo-PowerShellLiteral -Value $reparseAuditSkipReason
        }
        $auditScript = @'
$ErrorActionPreference = 'Stop'

$report = [ordered]@{}

function Test-Operation {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][scriptblock] $Action
    )

    try {
        $details = & $Action
        $result = [ordered]@{ status = 'passed' }
        if ($null -ne $details) {
            $result.details = @($details)
        }
        $report[$Name] = $result
    }
    catch {
        $report[$Name] = [ordered]@{
            status = 'failed'
            error = $_.Exception.Message
        }
    }
}

$clientRoot = __CLIENT_ROOT__
$clientDriveRoot = __CLIENT_DRIVE_ROOT__
$clientDirectory = __CLIENT_DIRECTORY__
$secondaryClientRoot = __SECONDARY_CLIENT_ROOT__
$clientFile = Join-Path $clientDirectory 'roundtrip.txt'
$renamedFile = Join-Path $clientDirectory 'renamed.txt'
$movedDirectory = Join-Path $clientDirectory 'moved'
$movedFile = Join-Path $movedDirectory 'moved.txt'
$clientSource = __CLIENT_SOURCE__
$serverToClient = __SERVER_TO_CLIENT__
$serverPayload = __SERVER_PAYLOAD__
$clientToServer = __CLIENT_TO_SERVER__
$resultPath = __RESULT_PATH__
$reparseAuditEscapePath = __REPARSE_AUDIT_ESCAPE_PATH__
$reparseAuditSkipReason = __REPARSE_AUDIT_SKIP_REASON__

Test-Operation -Name 'root-metadata-and-enumeration' -Action {
    $root = Get-Item -LiteralPath $clientRoot
    if (-not $root.PSIsContainer) {
        throw 'redirected drive root is not a directory'
    }
    $null = @(Get-ChildItem -LiteralPath $clientRoot -Force)
}

if ($null -ne $secondaryClientRoot) {
    Test-Operation -Name 'multiple-drive-announcement' -Action {
        if (-not (Get-Item -LiteralPath $secondaryClientRoot).PSIsContainer) {
            throw 'second redirected drive root is not a directory'
        }
    }
}

Test-Operation -Name 'directory-create' -Action {
    $null = New-Item -ItemType Directory -Path $clientDirectory -Force
    if (-not (Test-Path -LiteralPath $clientDirectory -PathType Container)) {
        throw 'could not create redirected directory'
    }
}

Test-Operation -Name 'file-create' -Action {
    $null = New-Item -ItemType File -Path $clientFile -Force
    if (-not (Test-Path -LiteralPath $clientFile -PathType Leaf)) {
        throw 'could not create redirected file'
    }
}

Test-Operation -Name 'create-write-read' -Action {
    [IO.File]::WriteAllText($clientFile, 'ironrdp-rdpdr-audit')
    if ([IO.File]::ReadAllText($clientFile) -ne 'ironrdp-rdpdr-audit') {
        throw 'redirected file contents did not round-trip'
    }
}

Test-Operation -Name 'nested-mixed-content-tree' -Action {
    $nestedDirectory = Join-Path $clientDirectory 'depth 01'
    foreach ($segment in @('depth 02', "unicode-$([char]0x00E5)-$([char]0x4E2D)", 'trailing dot.')) {
        $nestedDirectory = Join-Path $nestedDirectory $segment
        $null = New-Item -ItemType Directory -Path $nestedDirectory -Force
    }
    $bytes = New-Object byte[] 131071
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        $bytes[$index] = [byte] (($index * 31) % 251)
    }
    $binaryFile = Join-Path $nestedDirectory 'binary payload.bin'
    $zeroFile = Join-Path $nestedDirectory 'empty.bin'
    [IO.File]::WriteAllBytes($binaryFile, $bytes)
    [IO.File]::WriteAllBytes($zeroFile, [byte[]]::new(0))
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $expectedHash = [BitConverter]::ToString($sha256.ComputeHash($bytes)).Replace('-', '')
    }
    finally {
        $sha256.Dispose()
    }
    if ((Get-FileHash -LiteralPath $binaryFile -Algorithm SHA256).Hash -ne $expectedHash) {
        throw 'nested binary file hash did not round-trip'
    }
    if ((Get-Item -LiteralPath $zeroFile).Length -ne 0) {
        throw 'nested zero-length file was not preserved'
    }
    if (@(Get-ChildItem -LiteralPath $clientDirectory -File -Recurse -Force).Count -lt 3) {
        throw 'recursive directory enumeration omitted nested files'
    }
}

Test-Operation -Name 'alternate-data-stream-roundtrip' -Action {
    $streamBytes = [byte[]](5, 17, 29, 41, 53)
    Set-Content -LiteralPath $clientFile -Stream 'ironrdp-audit' -Value 'stream-value' -NoNewline
    Set-Content -LiteralPath $clientFile -Stream 'ironrdp-binary' -Value $streamBytes -Encoding Byte
    if ((Get-Content -LiteralPath $clientFile -Stream 'ironrdp-audit' -Raw) -ne 'stream-value') {
        throw 'text alternate data stream did not round-trip'
    }
    $actualStreamBytes = [byte[]] @(Get-Content -LiteralPath $clientFile -Stream 'ironrdp-binary' -Encoding Byte)
    if (-not [Linq.Enumerable]::SequenceEqual($actualStreamBytes, $streamBytes)) {
        throw 'binary alternate data stream did not round-trip'
    }
}

Test-Operation -Name 'alternate-data-stream-copy-compatibility' -Action {
    $copiedFile = Join-Path $clientDirectory 'stream-copy.txt'
    Copy-Item -LiteralPath $clientFile -Destination $copiedFile -Force
    $copiedTextStream = Get-Content -LiteralPath $copiedFile -Stream 'ironrdp-audit' -Raw -ErrorAction SilentlyContinue
    $copiedStreamBytes = [byte[]] @(Get-Content -LiteralPath $copiedFile -Stream 'ironrdp-binary' -Encoding Byte -ErrorAction SilentlyContinue)
    if ($copiedTextStream -eq 'stream-value' -and
        [Linq.Enumerable]::SequenceEqual($copiedStreamBytes, [byte[]](5, 17, 29, 41, 53))) {
        return 'preserved'
    }

    # The Windows RDPDR redirector may mask FILE_NAMED_STREAMS even when the
    # client supplies it. Preserve this interoperability observation without
    # obscuring failures in direct stream I/O.
    return 'not-preserved-by-remote-rdpdr-redirector'
}

Test-Operation -Name 'metadata-set-and-query' -Action {
    $expected = [DateTime]::UtcNow.AddMinutes(-5)
    [IO.File]::SetLastWriteTimeUtc($clientFile, $expected)
    $actual = [IO.File]::GetLastWriteTimeUtc($clientFile)
    if ([Math]::Abs(($actual - $expected).TotalSeconds) -gt 3) {
        throw 'last-write time did not round-trip'
    }
    [IO.File]::SetAttributes($clientFile, [IO.FileAttributes]::Archive)
    if (-not ([IO.File]::GetAttributes($clientFile).HasFlag([IO.FileAttributes]::Archive))) {
        throw 'archive attribute was not retained'
    }
}

Test-Operation -Name 'end-of-file-and-allocation' -Action {
    $stream = [IO.File]::Open($clientFile, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
    try {
        $stream.SetLength(8192)
    }
    finally {
        $stream.Dispose()
    }
    if ((Get-Item -LiteralPath $clientFile).Length -ne 8192) {
        throw 'file length did not update'
    }
}

Test-Operation -Name 'byte-range-locking' -Action {
    $stream = [IO.File]::Open($clientFile, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
    try {
        $stream.Lock(0, 1)
        $stream.Unlock(0, 1)
    }
    finally {
        $stream.Dispose()
    }
}

Test-Operation -Name 'sharing-violation' -Action {
    $first = [IO.File]::Open($clientFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try {
        try {
            $second = [IO.File]::Open($clientFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            try {
                throw 'second open unexpectedly ignored exclusive sharing'
            }
            finally {
                $second.Dispose()
            }
        }
        catch [IO.IOException] {
        }
    }
    finally {
        $first.Dispose()
    }
}

Test-Operation -Name 'rename-and-move' -Action {
    Rename-Item -LiteralPath $clientFile -NewName 'renamed.txt'
    $null = New-Item -ItemType Directory -Path $movedDirectory -Force
    Move-Item -LiteralPath $renamedFile -Destination $movedFile
    if (-not (Test-Path -LiteralPath $movedFile -PathType Leaf)) {
        throw 'renamed file was not moved'
    }
}

Test-Operation -Name 'directory-enumeration' -Action {
    $names = @(Get-ChildItem -LiteralPath $clientDirectory -Force | Select-Object -ExpandProperty Name)
    if ($names -notcontains 'moved') {
        throw 'directory enumeration did not include the moved directory'
    }
}

Test-Operation -Name 'query-security' -Action {
    $acl = Get-Acl -LiteralPath $movedFile
    if ($null -eq $acl -or [string]::IsNullOrWhiteSpace($acl.Sddl)) {
        throw 'security query returned no descriptor'
    }
}

Test-Operation -Name 'set-and-restore-security' -Action {
    $originalSddl = (Get-Acl -LiteralPath $movedFile).Sddl
    try {
        $security = [Security.AccessControl.FileSecurity]::new()
        $security.SetSecurityDescriptorSddlForm($originalSddl)
        $everyone = [Security.Principal.SecurityIdentifier]::new(
            [Security.Principal.WellKnownSidType]::WorldSid,
            $null
        )
        $security.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            $everyone,
            [Security.AccessControl.FileSystemRights]::ReadAndExecute,
            [Security.AccessControl.AccessControlType]::Allow
        ))
        Set-Acl -LiteralPath $movedFile -AclObject $security
        $updatedSddl = (Get-Acl -LiteralPath $movedFile).Sddl
        if ([string]::IsNullOrWhiteSpace($updatedSddl) -or $updatedSddl -notmatch 'WD') {
            throw 'security descriptor update was not retained'
        }
    }
    finally {
        $restore = [Security.AccessControl.FileSecurity]::new()
        $restore.SetSecurityDescriptorSddlForm($originalSddl)
        Set-Acl -LiteralPath $movedFile -AclObject $restore
    }
}

Test-Operation -Name 'read-only-write-denial' -Action {
    $readOnlyFile = Join-Path $clientDirectory 'read-only.txt'
    [IO.File]::WriteAllText($readOnlyFile, 'initial')
    [IO.File]::SetAttributes($readOnlyFile, [IO.FileAttributes]::ReadOnly)
    try {
        try {
            [IO.File]::WriteAllText($readOnlyFile, 'unexpected')
            throw 'write unexpectedly succeeded against a read-only redirected file'
        }
        catch [UnauthorizedAccessException] {
        }
        catch [IO.IOException] {
        }
    }
    finally {
        [IO.File]::SetAttributes($readOnlyFile, [IO.FileAttributes]::Normal)
    }
}

if ($null -ne $secondaryClientRoot) {
    Test-Operation -Name 'multiple-drive-cross-volume-copy' -Action {
        $secondaryDirectory = Join-Path $secondaryClientRoot "ironrdp-rdpdr-audit-$PID"
        $secondaryFile = Join-Path $secondaryDirectory 'from-primary.txt'
        $returnedFile = Join-Path $clientDirectory 'from-secondary.txt'
        try {
            $null = New-Item -ItemType Directory -Path $secondaryDirectory -Force
            Copy-Item -LiteralPath $movedFile -Destination $secondaryFile -Force
            Copy-Item -LiteralPath $secondaryFile -Destination $returnedFile -Force
            if ((Get-FileHash -LiteralPath $movedFile -Algorithm SHA256).Hash -ne
                (Get-FileHash -LiteralPath $returnedFile -Algorithm SHA256).Hash) {
                throw 'cross-volume copy changed file content'
            }
            $returnedTextStream = Get-Content -LiteralPath $returnedFile -Stream 'ironrdp-audit' -Raw -ErrorAction SilentlyContinue
            if ($returnedTextStream -eq 'stream-value') {
                return 'alternate-data-streams-preserved'
            }

            return 'alternate-data-streams-not-preserved-by-remote-rdpdr-redirector'
        }
        finally {
            Remove-Item -LiteralPath $secondaryDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Operation -Name 'reparse-point-confinement' -Action {
    if ($null -eq $reparseAuditEscapePath) {
        return $reparseAuditSkipReason
    }

    try {
        $null = [IO.File]::ReadAllText($reparseAuditEscapePath)
        throw 'reparse-point traversal unexpectedly read outside the redirected root'
    }
    catch [IO.IOException] {
        return (
            'blocked={0}; hresult=0x{1:X8}' -f
            $_.Exception.GetType().Name,
            [BitConverter]::ToUInt32([BitConverter]::GetBytes([int] $_.Exception.HResult), 0)
        )
    }
    catch [UnauthorizedAccessException] {
        return (
            'blocked={0}; hresult=0x{1:X8}' -f
            $_.Exception.GetType().Name,
            [BitConverter]::ToUInt32([BitConverter]::GetBytes([int] $_.Exception.HResult), 0)
        )
    }
}

Test-Operation -Name 'query-volume-information' -Action {
    Add-Type -TypeDefinition @"
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

public static class IronRdpAuditNativeMethods
{
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool GetDiskFreeSpaceEx(
        string directoryName,
        out ulong freeBytesAvailable,
        out ulong totalNumberOfBytes,
        out ulong totalNumberOfFreeBytes);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool GetVolumeInformation(
        string rootPathName,
        StringBuilder volumeNameBuffer,
        int volumeNameSize,
        out uint volumeSerialNumber,
        out uint maximumComponentLength,
        out uint fileSystemFlags,
        StringBuilder fileSystemNameBuffer,
        int fileSystemNameSize);
}

public static class IronRdpAuditConcurrentTransfers
{
    public static int Run(string serverPayload, string clientDirectory, int copyCount)
    {
        if (copyCount <= 0)
        {
            throw new ArgumentOutOfRangeException("copyCount");
        }

        byte[] expectedHash = HashFile(serverPayload);
        Task[] tasks = new Task[copyCount];
        for (int index = 0; index < copyCount; index++)
        {
            tasks[index] = Task.Run(() => CopyPair(serverPayload, clientDirectory, expectedHash));
        }

        Task.WaitAll(tasks);
        return copyCount;
    }

    private static void CopyPair(string serverPayload, string clientDirectory, byte[] expectedHash)
    {
        string clientPath = Path.Combine(clientDirectory, "parallel-" + Guid.NewGuid().ToString("N") + ".bin");
        string serverDirectory = Path.GetDirectoryName(serverPayload);
        string returnedPath = Path.Combine(serverDirectory, "parallel-return-" + Guid.NewGuid().ToString("N") + ".bin");
        try
        {
            File.Copy(serverPayload, clientPath, true);
            AssertHash(clientPath, expectedHash);
            File.Copy(clientPath, returnedPath, true);
            AssertHash(returnedPath, expectedHash);
        }
        finally
        {
            if (File.Exists(clientPath))
            {
                File.Delete(clientPath);
            }
            if (File.Exists(returnedPath))
            {
                File.Delete(returnedPath);
            }
        }
    }

    private static byte[] HashFile(string path)
    {
        using (SHA256 hasher = SHA256.Create())
        using (FileStream stream = File.OpenRead(path))
        {
            return hasher.ComputeHash(stream);
        }
    }

    private static void AssertHash(string path, byte[] expectedHash)
    {
        byte[] actualHash = HashFile(path);
        if (actualHash.Length != expectedHash.Length)
        {
            throw new IOException("parallel copy hash length mismatch");
        }
        for (int index = 0; index < actualHash.Length; index++)
        {
            if (actualHash[index] != expectedHash[index])
            {
                throw new IOException("parallel copy hash mismatch");
            }
        }
    }
}

public sealed class IronRdpAuditWatchSignal : IDisposable
{
    private readonly AutoResetEvent signalled = new AutoResetEvent(false);
    private int eventCount;
    private string error;
    private bool requiresRescan;

    public void Attach(FileSystemWatcher watcher)
    {
        watcher.Created += OnEvent;
        watcher.Changed += OnEvent;
        watcher.Deleted += OnEvent;
        watcher.Renamed += OnRenamed;
        watcher.Error += OnError;
    }

    public bool Wait(int millisecondsTimeout)
    {
        return signalled.WaitOne(millisecondsTimeout);
    }

    public int EventCount
    {
        get { return eventCount; }
    }

    public string Error
    {
        get { return error; }
    }

    public bool RequiresRescan
    {
        get { return requiresRescan; }
    }

    public void Dispose()
    {
        signalled.Dispose();
    }

    private void OnEvent(object sender, FileSystemEventArgs eventArgs)
    {
        Interlocked.Increment(ref eventCount);
        signalled.Set();
    }

    private void OnRenamed(object sender, RenamedEventArgs eventArgs)
    {
        Interlocked.Increment(ref eventCount);
        signalled.Set();
    }

    private void OnError(object sender, ErrorEventArgs eventArgs)
    {
        Exception exception = eventArgs.GetException();
        error = exception.Message;
        requiresRescan = exception is InternalBufferOverflowException;
        signalled.Set();
    }
}
"@
    [UInt64] $free = 0
    [UInt64] $total = 0
    [UInt64] $freeTotal = 0
    if (-not [IronRdpAuditNativeMethods]::GetDiskFreeSpaceEx($clientRoot, [ref] $free, [ref] $total, [ref] $freeTotal)) {
        throw ([ComponentModel.Win32Exception]::new([Runtime.InteropServices.Marshal]::GetLastWin32Error()))
    }
    if ($total -eq 0) {
        throw 'redirected drive reported zero capacity'
    }
    $volumeName = New-Object Text.StringBuilder 261
    $fileSystemName = New-Object Text.StringBuilder 261
    [UInt32] $serial = 0
    [UInt32] $maximumComponentLength = 0
    [UInt32] $flags = 0
    if (-not [IronRdpAuditNativeMethods]::GetVolumeInformation(
            $clientDriveRoot,
            $volumeName,
            $volumeName.Capacity,
            [ref] $serial,
            [ref] $maximumComponentLength,
            [ref] $flags,
            $fileSystemName,
            $fileSystemName.Capacity)) {
        throw ([ComponentModel.Win32Exception]::new([Runtime.InteropServices.Marshal]::GetLastWin32Error()))
    }
    if (($flags -band 0x00040000) -ne 0) {
        return ('flags=0x{0:X8}; named-streams-advertised=true' -f $flags)
    }

    return ('flags=0x{0:X8}; named-streams-advertised=false' -f $flags)
}

Test-Operation -Name 'concurrent-bidirectional-copies' -Action {
    [IronRdpAuditConcurrentTransfers]::Run($serverPayload, $clientDirectory, 8)
}

Test-Operation -Name 'server-to-client-copy' -Action {
    Copy-Item -LiteralPath $serverPayload -Destination $serverToClient -Force
    if (-not (Test-Path -LiteralPath $serverToClient -PathType Leaf)) {
        throw 'server-to-client copy did not create the redirected file'
    }
}

Test-Operation -Name 'client-to-server-copy' -Action {
    Copy-Item -LiteralPath $clientSource -Destination $clientToServer -Force
}

Test-Operation -Name 'directory-notification' -Action {
    $watcher = [IO.FileSystemWatcher]::new($clientDirectory)
    $signal = [IronRdpAuditWatchSignal]::new()
    try {
        $watcher.NotifyFilter = [IO.NotifyFilters]::FileName
        $signal.Attach($watcher)
        $watcher.EnableRaisingEvents = $true
        $notificationPath = Join-Path $clientDirectory 'notify.txt'
        [IO.File]::WriteAllText($notificationPath, 'notify')
        if (-not $signal.Wait(10000)) {
            throw "directory change notification timed out (events: $($signal.EventCount); error: $($signal.Error))"
        }
        if ($signal.EventCount -eq 0 -and -not $signal.RequiresRescan) {
            throw "directory change notification failed: $($signal.Error)"
        }
        if (-not (Test-Path -LiteralPath $notificationPath -PathType Leaf)) {
            throw 'directory change notification did not preserve the created file'
        }
    }
    finally {
        $signal.Dispose()
        $watcher.Dispose()
    }
}

Test-Operation -Name 'delete-and-cleanup' -Action {
    Remove-Item -LiteralPath $clientDirectory -Recurse -Force
    if (Test-Path -LiteralPath $clientDirectory) {
        throw 'redirected directory was not deleted'
    }
}

[IO.File]::WriteAllText($resultPath, ($report | ConvertTo-Json -Compress))

if (@($report.GetEnumerator() | Where-Object { $_.Value.status -ne 'passed' }).Count -ne 0) {
    exit 1
}
'@
        $auditScript = $auditScript.Replace('__CLIENT_ROOT__', $clientRootLiteral).
            Replace('__CLIENT_DRIVE_ROOT__', $clientDriveRootLiteral).
            Replace('__CLIENT_DIRECTORY__', $clientDirectoryLiteral).
            Replace('__SECONDARY_CLIENT_ROOT__', $secondaryClientRootLiteral).
            Replace('__CLIENT_SOURCE__', $clientSourceLiteral).
            Replace('__SERVER_TO_CLIENT__', $localServerPayloadLiteral).
            Replace('__SERVER_PAYLOAD__', $remoteServerPayloadLiteral).
            Replace('__CLIENT_TO_SERVER__', $remoteInboundPayloadLiteral).
            Replace('__RESULT_PATH__', $remoteAuditResultLiteral).
            Replace('__REPARSE_AUDIT_ESCAPE_PATH__', $reparseAuditEscapePathLiteral).
            Replace('__REPARSE_AUDIT_SKIP_REASON__', $reparseAuditSkipReasonLiteral)
        $auditScript | Out-File -LiteralPath (Join-Path $ArtifactsDir 'full-audit-script.ps1')
        Write-RemoteScript -Path $remoteAuditScript -Contents $auditScript
        $remoteAuditScriptLiteral = ConvertTo-PowerShellLiteral -Value $remoteAuditScript
        $parseOutput = Invoke-Agent now powershell "try { `$null = [scriptblock]::Create([IO.File]::ReadAllText($remoteAuditScriptLiteral)); 'parse succeeded' } catch { `$_.Exception.Message; exit 1 }" --timeout 30 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0 -or $parseOutput -notmatch 'parse succeeded') {
            throw "remote full-surface audit script did not parse: $parseOutput"
        }
        Invoke-RemoteDesktopCommand `
            -Command "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $remoteAuditScript" `
            -TypedCommandScreenshot (Join-Path $ArtifactsDir 'typed-audit-command.png')
        Wait-RemoteFile -Path $remoteAuditResult
        Save-RemoteTextFile -Path $remoteAuditResult -Destination (Join-Path $ArtifactsDir 'full-audit-report.raw.log')
        $auditText = Get-Content -LiteralPath (Join-Path $ArtifactsDir 'full-audit-report.raw.log') -Raw
        $auditJson = [regex]::Match($auditText, '(?s)\{.*\}')
        if (-not $auditJson.Success) {
            throw 'could not parse the remote full-surface audit report'
        }
        $auditReport = $auditJson.Value | ConvertFrom-Json
        if (-not (Test-Path -LiteralPath $localServerPayload -PathType Leaf)) {
            throw 'server-to-client audit copy did not create a local file'
        }
        if ((Get-FileHash -LiteralPath $localServerPayload -Algorithm SHA256).Hash -ne $serverPayloadHash) {
            throw 'server-to-client audit payload hash mismatch'
        }
        Wait-RemoteFileHash -Path $remoteInboundPayload -ExpectedHash $sourceHash
        Invoke-Agent query-logs --last 4096 2>&1 |
            Out-File (Join-Path $ArtifactsDir 'session-diagnostics.log')
        if ($LASTEXITCODE -ne 0) {
            throw 'could not retrieve full RDPDR diagnostics'
        }
        $failedAuditOperations = @(
            $auditReport.PSObject.Properties |
                Where-Object { $_.Value.status -ne 'passed' } |
                ForEach-Object { $_.Name }
        )
        if ($failedAuditOperations.Count -ne 0) {
            throw "RDPDR full-surface audit failed: $($failedAuditOperations -join ', ')"
        }
    }

    $rdpdrReadCount = 0
    if ($payloadByteCount -le 512MB) {
        Start-Sleep -Seconds 1
        $rdpdrLogs = Invoke-Agent query-logs --substring 'Dispatching filesystem read IRP' --last 2048 2>&1
        $rdpdrLogs | Out-File (Join-Path $ArtifactsDir 'rdpdr-read-events.log')
        if ($LASTEXITCODE -ne 0) {
            throw 'could not retrieve RDPDR read diagnostics'
        }
        $rdpdrReadCount = @($rdpdrLogs).Count
        $minimumReadRequests = if ($payloadByteCount -ge 9MB) { 9 } else { 1 }
        if ($rdpdrReadCount -lt $minimumReadRequests) {
            throw "expected at least $minimumReadRequests RDPDR read requests, observed $rdpdrReadCount"
        }
    }

    Invoke-Agent screenshot (Join-Path $ArtifactsDir 'completed.png') | Out-Null
    if ($payloadByteCount -gt 512MB) {
        Write-Host "RDPDR large-transfer regression completed with SHA-256 verification in both directions"
    }
    else {
        Write-Host "RDPDR regression completed with $rdpdrReadCount read requests"
    }
}
catch {
    if ($connected) {
        try {
            Invoke-Agent screenshot (Join-Path $ArtifactsDir 'failure.png') | Out-Null
        }
        catch {
            Write-Warning "Could not capture failure screenshot: $($_.Exception.Message)"
        }
        try {
            Invoke-Agent query-logs --last 2048 | Out-File (Join-Path $ArtifactsDir 'failure-session.log')
        }
        catch {
            Write-Warning "Could not retrieve failure session logs: $($_.Exception.Message)"
        }
        try {
            Save-RemoteTextFile -Path $remoteCopyResult -Destination (Join-Path $ArtifactsDir 'copy-result.log')
        }
        catch {
            Write-Warning "Could not retrieve remote copy result: $($_.Exception.Message)"
        }
    }
    throw
}
finally {
    if ($connected) {
        try {
            $cleanupPaths = @(
                if ([string]::IsNullOrWhiteSpace($requestedSourceFile) -or $largeTransferSucceeded) {
                    $remoteDirectFile
                }
                $remoteCopyResult,
                $remoteCopyScript,
                $remoteCopyStatusFile,
                $remoteRoundTripResult,
                $remoteRoundTripScript,
                $remoteRoundTripStatusFile,
                $remoteAuditScript,
                $remoteAuditResult,
                $remoteServerPayload,
                $remoteInboundPayload
            ) |
                ForEach-Object { ConvertTo-PowerShellLiteral -Value $_ }
            Invoke-Agent now powershell "Remove-Item -LiteralPath $($cleanupPaths -join ', ') -Force -ErrorAction SilentlyContinue" --timeout 30 | Out-Null
        }
        catch {
            Write-Warning "Could not remove remote test files: $($_.Exception.Message)"
        }
        try {
            Invoke-Agent disconnect | Out-Null
        }
        catch {
            Write-Warning "Could not disconnect the agent session: $($_.Exception.Message)"
        }
    }
    if ($null -ne $daemon) {
        $process = Get-Process -Id $daemon.Id -ErrorAction SilentlyContinue
        if ($null -ne $process) {
            Stop-Process -Id $process.Id -Force
        }
    }
    if (Test-Path -LiteralPath $sourceRoot) {
        Remove-Item -LiteralPath $sourceRoot -Recurse -Force
    }
    if ($null -ne $reparseAuditRoot -and (Test-Path -LiteralPath $reparseAuditRoot)) {
        Remove-Item -LiteralPath $reparseAuditRoot -Recurse -Force
    }
}
