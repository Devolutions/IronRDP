param(
    [Parameter(Mandatory = $true)]
    [string]$ComputerName,

    [Parameter(Mandatory = $true)]
    [pscredential]$Credential,

    [Parameter()]
    [datetime]$Since = (Get-Date).AddHours(-1),

    [Parameter()]
    [string]$UserLike
)

$ErrorActionPreference = 'Stop'

function Convert-EventToDataMap {
    param([Parameter(Mandatory = $true)] [System.Diagnostics.Eventing.Reader.EventRecord]$Event)

    $xml = [xml]$Event.ToXml()
    $map = @{}

    foreach ($d in $xml.Event.EventData.Data) {
        $name = [string]$d.Name
        if ([string]::IsNullOrWhiteSpace($name)) {
            continue
        }

        $map[$name] = [string]$d.'#text'
    }

    return $map
}

function Format-TableString {
    param([Parameter(Mandatory = $true)] $InputObject)
    if ($null -eq $InputObject) {
        return '(none)'
    }

    $s = ($InputObject | Format-Table -AutoSize | Out-String -Width 400).TrimEnd()
    if ([string]::IsNullOrWhiteSpace($s)) { '(none)' } else { $s }
}

function Get-FirstMapValue {
    param(
        [Parameter(Mandatory = $true)] [hashtable]$Map,
        [Parameter(Mandatory = $true)] [string[]]$Keys
    )

    foreach ($k in $Keys) {
        if ($Map.ContainsKey($k) -and -not [string]::IsNullOrWhiteSpace($Map[$k])) {
            return $Map[$k]
        }
    }

    return $null
}

$result = Invoke-Command -ComputerName $ComputerName -Credential $Credential -ArgumentList $Since, $UserLike -ScriptBlock {
    param($since, $userLike)

    $ErrorActionPreference = 'Stop'

    function Convert-EventToDataMap {
        param([Parameter(Mandatory = $true)] [System.Diagnostics.Eventing.Reader.EventRecord]$Event)

        $xml = [xml]$Event.ToXml()
        $map = @{}

        foreach ($d in $xml.Event.EventData.Data) {
            $name = [string]$d.Name
            if ([string]::IsNullOrWhiteSpace($name)) {
                continue
            }

            $map[$name] = [string]$d.'#text'
        }

        return $map
    }

    function Get-FirstMapValue {
        param(
            [Parameter(Mandatory = $true)] [hashtable]$Map,
            [Parameter(Mandatory = $true)] [string[]]$Keys
        )

        foreach ($k in $Keys) {
            if ($Map.ContainsKey($k) -and -not [string]::IsNullOrWhiteSpace($Map[$k])) {
                return $Map[$k]
            }
        }

        return $null
    }

    # Avoid `quser` inside PSRemoting: it can write directly to the host stream and force exit code 1.
    # Session existence is confirmed via LSM Operational events (notably event ID 21 with a SessionID).
    $quserText = '(skipped)'

    $logons = @()
    try {
        $logons = Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4624; StartTime = $since } -ErrorAction Stop
    }
    catch {
        $logons = @()
    }

    $logons = $logons |
        ForEach-Object {
            $m = Convert-EventToDataMap -Event $_
            if ($m.LogonType -ne '10') {
                return
            }

            $user = ($m.TargetDomainName + '\' + $m.TargetUserName)
            if ($userLike -and ($user -notlike ('*' + $userLike + '*'))) {
                return
            }

            [pscustomobject]@{
                TimeCreated = $_.TimeCreated
                User        = $user
                LogonId     = $m.TargetLogonId
                IpAddress   = $m.IpAddress
                IpPort      = $m.IpPort
                AuthPkg     = $m.AuthenticationPackageName
                LogonProc   = $m.LogonProcessName
            }
        } |
        Where-Object { $_ } |
        Sort-Object TimeCreated -Descending

    $failures = @()
    try {
        $failures = Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4625; StartTime = $since } -ErrorAction Stop
    }
    catch {
        $failures = @()
    }

    $failures = $failures |
        ForEach-Object {
            $m = Convert-EventToDataMap -Event $_
            if ($m.LogonType -ne '10') {
                return
            }

            $user = ($m.TargetDomainName + '\' + $m.TargetUserName)
            if ($userLike -and ($user -notlike ('*' + $userLike + '*'))) {
                return
            }

            [pscustomobject]@{
                TimeCreated = $_.TimeCreated
                User        = $user
                Status      = $m.Status
                SubStatus   = $m.SubStatus
                IpAddress   = $m.IpAddress
                IpPort      = $m.IpPort
                Proc        = $m.ProcessName
            }
        } |
        Where-Object { $_ } |
        Sort-Object TimeCreated -Descending

    $latestLogon = $logons | Select-Object -First 1

    $logoffs = @()
    if ($null -ne $latestLogon) {
        $logoffs = @()
        try {
            $logoffs = Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4634; StartTime = $since } -ErrorAction Stop
        }
        catch {
            $logoffs = @()
        }

        $logoffs = $logoffs |
            ForEach-Object {
                $m = Convert-EventToDataMap -Event $_
                if ($m.LogonType -ne '10') { return }
                if ($m.TargetLogonId -ne $latestLogon.LogonId) { return }
                [pscustomobject]@{
                    TimeCreated = $_.TimeCreated
                    User        = ($m.TargetDomainName + '\' + $m.TargetUserName)
                    LogonId     = $m.TargetLogonId
                }
            } |
            Where-Object { $_ } |
            Sort-Object TimeCreated -Descending
    }

    $lsm = @()
    try {
        $lsm = Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-TerminalServices-LocalSessionManager/Operational'; Id = 21, 22, 23, 24, 25; StartTime = $since } -ErrorAction Stop
    }
    catch {
        $lsm = @()
    }

    $lsm = $lsm |
        ForEach-Object {
            $m = Convert-EventToDataMap -Event $_
            $user = Get-FirstMapValue -Map $m -Keys @('User', 'UserName', 'UserNameString')
            if ($userLike -and $user -and ($user -notlike ('*' + $userLike + '*'))) {
                return
            }

            [pscustomobject]@{
                TimeCreated = $_.TimeCreated
                Id          = $_.Id
                SessionId   = Get-FirstMapValue -Map $m -Keys @('SessionID', 'SessionId', 'Session')
                User        = $user
                Address     = Get-FirstMapValue -Map $m -Keys @('Address', 'ClientAddress', 'SourceNetworkAddress')
                Reason      = Get-FirstMapValue -Map $m -Keys @('Reason', 'DisconnectReason')
                Message     = $_.Message
            }
        } |
        Where-Object { $_ } |
        Sort-Object TimeCreated -Descending

    $correlatedLsm21 = $null
    if ($null -ne $latestLogon) {
        $latestUsername = $latestLogon.User.Split('\\')[-1]
        $candidates = $lsm | Where-Object { $_.Id -eq 21 -and $_.User -and ($_.User -like ('*' + $latestUsername + '*')) }
        $correlatedLsm21 = $candidates |
            Sort-Object { [math]::Abs(($_.TimeCreated - $latestLogon.TimeCreated).TotalSeconds) } |
            Select-Object -First 1
    }

    [pscustomobject]@{
        Host              = $env:COMPUTERNAME
        Now               = Get-Date
        Since             = $since
        Quser             = $quserText
        Latest4624Type10  = $latestLogon
        CorrelatedLsm21   = $correlatedLsm21
        Recent4624Type10  = ($logons | Select-Object -First 10)
        Recent4625Type10  = ($failures | Select-Object -First 10)
        Recent4634Type10  = ($logoffs | Select-Object -First 5)
        RecentLsm         = ($lsm | Select-Object -First 30)
    }
}

Write-Output ('Host:  ' + $result.Host)
Write-Output ('Now:   ' + ($result.Now.ToString('s')))
Write-Output ('Since: ' + ($result.Since.ToString('s')))
Write-Output ''
Write-Output '=== QUSER (current sessions) ==='
Write-Output ($result.Quser)
Write-Output ''
Write-Output '=== Latest Security 4624 (LogonType=10) ==='
if ($null -eq $result.Latest4624Type10) {
    Write-Output '(none found)'
}
else {
    $result.Latest4624Type10 | Format-List
}
Write-Output ''
Write-Output '=== Correlated LSM 21 (session created/logon) ==='
if ($null -eq $result.CorrelatedLsm21) {
    Write-Output '(none found)'
}
else {
    $result.CorrelatedLsm21 | Format-List
}
Write-Output ''
Write-Output '=== Recent Security 4624 (LogonType=10) ==='
Write-Output (Format-TableString -InputObject $result.Recent4624Type10)
Write-Output ''
Write-Output '=== Recent Security 4625 (LogonType=10) ==='
Write-Output (Format-TableString -InputObject $result.Recent4625Type10)
Write-Output ''
Write-Output '=== Recent Security 4634 (LogonType=10, matching LogonId when possible) ==='
Write-Output (Format-TableString -InputObject $result.Recent4634Type10)
Write-Output ''
Write-Output '=== Recent LSM Operational (21/22/23/24/25) ==='
Write-Output (Format-TableString -InputObject ($result.RecentLsm | Select-Object TimeCreated, Id, SessionId, User, Address, Reason))
Write-Output ''
Write-Output '=== Recent LSM Messages (first 8) ==='
if (-not $result.RecentLsm) {
    Write-Output '(none)'
}
else {
    ($result.RecentLsm | Select-Object -First 8 | ForEach-Object { ($_.TimeCreated.ToString('s') + ' id=' + $_.Id + ' ' + $_.Message) })
}

exit 0