function Select-CaptureSession {
    param([object[]]$Rows, [int[]]$OwnedSessionIds, [bool]$PreferIrdpTcpSession)
    $eligible = @($Rows | Where-Object { $_.Id -in $OwnedSessionIds -and $_.State -in @('Active', 'Conn') })
    if ($PreferIrdpTcpSession) {
        $preferred = $eligible | Where-Object { $_.SessionName -match '(?i)^irdp-tcp#' } |
            Sort-Object Id -Descending | Select-Object -First 1
        if ($null -ne $preferred) { return $preferred }
    }
    return $eligible | Sort-Object Id -Descending | Select-Object -First 1
}

function Assert-CaptureSession {
    param([int]$ExpectedSessionId, [string]$ExpectedUserSid, [object]$Capture)
    if ($Capture.SessionId -ne $ExpectedSessionId -or $Capture.UserSid -ne $ExpectedUserSid) {
        throw 'Capture was produced by a different Windows session or user'
    }
}
