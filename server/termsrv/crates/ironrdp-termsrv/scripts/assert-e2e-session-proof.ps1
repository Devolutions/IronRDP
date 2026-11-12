param([Parameter(Mandatory = $true)][hashtable]$Evidence)
Set-StrictMode -Version Latest
if ($Evidence.preflightBlockedConnectionAttempt -and ($Evidence.hardPreflightIssues.Count -gt 0)) {
    Write-Host "STRICT RESULT: BLOCKED" -ForegroundColor Yellow
    foreach ($issue in $Evidence.hardPreflightIssues) {
        Write-Host "  - $issue" -ForegroundColor Yellow
    }

    throw "strict session proof blocked by preflight gate failure(s)"
}

$strictFailures = New-Object System.Collections.Generic.List[string]

if (-not $Evidence.screenshotExists) {
    $strictFailures.Add('screenshot not produced')
} elseif ($Evidence.screenshotFileInfo.Length -le 1000) {
    $strictFailures.Add("screenshot too small ($($Evidence.screenshotFileInfo.Length) bytes)")
} elseif (-not $Evidence.analysisSucceeded) {
    $strictFailures.Add('screenshot analysis failed')
} elseif (-not $Evidence.hasMeaningfulContent) {
    $strictFailures.Add('screenshot content is blank or uniform')
}

if (-not $Evidence.remoteLogCollectionSucceeded) {
    $strictFailures.Add('remote log collection failed')
}

if ($Evidence.hardPreflightIssues.Count -gt 0) {
    foreach ($issue in $Evidence.hardPreflightIssues) {
        $strictFailures.Add("preflight gate failed: $issue")
    }
}

if ($Evidence.Mode -eq 'Provider') {
    if (-not $Evidence.evidenceCorrelated) {
        $strictFailures.Add("connection evidence is not correlated: $($Evidence.evidenceCorrelationReason)")
    }
    $hasSecurityType10Signal = $false
    if (($null -ne $Evidence.securityLogonType10Count) -and ($Evidence.securityLogonType10Count -ge 1)) {
        $hasSecurityType10Signal = $true
    }

    if (-not $hasSecurityType10Signal) {
        $strictFailures.Add("mandatory logon proof missing: Security 4624 LogonType=10 not observed for configured user '$($Evidence.RdpUsername)' (count=$($Evidence.securityLogonType10Count))")
    }

    if ($null -eq $Evidence.termsrvFallbackMarkerCount) {
        $strictFailures.Add('termsrv fallback marker count was not collected')
    } elseif ($Evidence.termsrvFallbackMarkerCount -gt 0) {
        $strictFailures.Add("termsrv fallback markers detected (count=$($Evidence.termsrvFallbackMarkerCount))")
    }

    if ($null -eq $Evidence.providerSessionProofMarkerCount) {
        $strictFailures.Add('provider session proof marker count was not collected')
    } elseif ($Evidence.providerSessionProofMarkerCount -lt 1) {
        $strictFailures.Add('provider session proof markers were not observed')
    }

    if ($null -eq $Evidence.notifyCommandProcessCreatedCount) {
        $strictFailures.Add('shell transition marker count was not collected (NotifyCommandProcessCreated)')
    } elseif ($Evidence.notifyCommandProcessCreatedCount -lt 1) {
        $strictFailures.Add('shell transition marker missing: NotifyCommandProcessCreated was not observed')
    }

    if ($null -eq $Evidence.termsrvSessionProofMarkerCount) {
        $strictFailures.Add('termsrv session proof marker count was not collected')
    } elseif ($Evidence.termsrvSessionProofMarkerCount -lt 1) {
        $strictFailures.Add('termsrv session proof markers were not observed')
    }

    if (-not $Evidence.guiTargetSessionResolved) {
        $strictFailures.Add('GUI session gate missing: could not resolve target capture session id from provider/termsrv markers')
    } elseif (-not $Evidence.guiTargetSessionProcessProof) {
        $strictFailures.Add("GUI session gate failed: explorer.exe not observed in target session $($Evidence.guiTargetSessionId) (source=$($Evidence.guiTargetSessionSource), explorer=$($Evidence.guiTargetSessionExplorerCount), gui_processes=$($Evidence.guiTargetSessionGuiProcessCount), winlogon=$($Evidence.guiTargetSessionWinlogonCount), logonui=$($Evidence.guiTargetSessionLogonUiCount))")
    }

    if (-not $Evidence.providerHardwareIdCustom) {
        $strictFailures.Add('custom IDD hardware id was not confirmed in runtime state')
    }

    if ($Evidence.providerGetHardwareIdCount -lt 1) {
        $strictFailures.Add('custom IDD hardware id callback was not observed in provider logs')
    }

    if ($Evidence.providerOnDriverLoadCount -lt 1) {
        $strictFailures.Add('OnDriverLoad(session, nonzero_handle) was not observed in provider logs')
    }

    if (-not $Evidence.providerCustomVideoSourceSelected) {
        $strictFailures.Add('provider did not return the OnDriverLoad handle from GetVideoHandle')
    }

    if ($Evidence.providerVideoHandleDriverLoadCount -lt 1) {
        $strictFailures.Add('provider did not log a GetVideoHandle selection sourced from OnDriverLoad')
    }

    if ($Evidence.providerManualVideoSourceSelected) {
        $strictFailures.Add('provider selected a manual custom-device video source instead of the OnDriverLoad handle')
    }

    if ($Evidence.providerFallbackVideoSourceSelected) {
        $strictFailures.Add('provider fell back to rdp_video_miniport')
    }

    if ($Evidence.providerAssumePresentCount -gt 0) {
        $strictFailures.Add("provider emitted synthetic ASSUME_PRESENT markers (count=$($Evidence.providerAssumePresentCount))")
    }

    if ($Evidence.iddMonitorArrivalMarkerCount -lt 1) {
        $strictFailures.Add('IDD monitor-arrival marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddDisplayConfigUpdateRequestCount -lt 1) {
        $strictFailures.Add('IddCxDisplayConfigUpdate request marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddDisplayConfigUpdateSuccessCount -lt 1) {
        $strictFailures.Add('IddCxDisplayConfigUpdate success marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddCommitModesMarkerCount -lt 1) {
        $strictFailures.Add('CommitModes marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddCommitModesActivePathCount -lt 1) {
        $strictFailures.Add('CommitModes never reported an active display path in the custom IDD debug trace')
    }

    if ($Evidence.iddSwapchainAssignedCount -lt 1) {
        $strictFailures.Add('swapchain assignment marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddSessionReadyMarkerCount -lt 1) {
        $strictFailures.Add('IDD session-ready marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddFirstFrameMarkerCount -lt 1) {
        $strictFailures.Add('IDD first-frame marker was not observed in the custom IDD debug trace')
    }

    if ($Evidence.iddDumpCount -lt 1) {
        $strictFailures.Add('no custom IDD bitmap dumps were downloaded')
    } elseif ($Evidence.guiTargetSessionResolved -and (-not $Evidence.iddTargetSessionHasGraphics)) {
        $observedBitmapSessions = if ($Evidence.iddObservedSessionIds.Count -gt 0) { $Evidence.iddObservedSessionIds -join ',' } else { 'none' }
        $strictFailures.Add("graphics-session proof missing: no custom IDD bitmap dump matched target session id $($Evidence.guiTargetSessionId) (observed_idd_sessions=$observedBitmapSessions)")
    }

    if (-not $Evidence.type10IddGraphicsSessionConfirmed) {
        $strictFailures.Add("mandatory type10+custom-idd-graphics proof missing: Security4624Type10=$($Evidence.securityLogonType10Count) target_session=$($Evidence.guiTargetSessionId) idd_bitmap_matches=$($Evidence.iddTargetSessionMatchCount)")
    }

    if ($null -eq $Evidence.providerVideoHandleMarkerCount) {
        $strictFailures.Add('IDD provider video-handle marker count was not collected')
    }

    if ($null -eq $Evidence.termsrvIddMarkerCount) {
        $strictFailures.Add('IDD termsrv marker count was not collected')
    }

    if ($null -eq $Evidence.providerLogonPolicyMarkerCount) {
        $strictFailures.Add('provider logon-policy marker count was not collected')
    }

    $hasIddDiagnosticsSignal = (($Evidence.providerOnDriverLoadCount -ge 1) -or $Evidence.iddDriverLoadedNotified) -or
        (($null -ne $Evidence.iddWddmEnabledSignalCount) -and ($Evidence.iddWddmEnabledSignalCount -ge 1)) -or
        (($null -ne $Evidence.providerVideoHandleMarkerCount) -and ($Evidence.providerVideoHandleMarkerCount -ge 1)) -or
        (($null -ne $Evidence.termsrvIddMarkerCount) -and ($Evidence.termsrvIddMarkerCount -ge 1)) -or
        ($Evidence.iddDisplayConfigUpdateRequestCount -ge 1) -or
        ($Evidence.iddCommitModesMarkerCount -ge 1) -or
        ($Evidence.iddSwapchainAssignedCount -ge 1)
    if (-not $hasIddDiagnosticsSignal) {
        $strictFailures.Add("IDD diagnostics gate missing (OnDriverLoad=$($Evidence.providerOnDriverLoadCount), ProviderEnableWddmIdd=$($Evidence.iddWddmEnabledSignalCount), ProviderVideoHandleMarkers=$($Evidence.providerVideoHandleMarkerCount), TermsrvIddMarkers=$($Evidence.termsrvIddMarkerCount), DisplayConfigRequest=$($Evidence.iddDisplayConfigUpdateRequestCount), CommitModes=$($Evidence.iddCommitModesMarkerCount), SwapchainAssigned=$($Evidence.iddSwapchainAssignedCount))")
    }

}

if ($strictFailures.Count -gt 0) {
    Write-Host "STRICT RESULT: FAIL" -ForegroundColor Red
    foreach ($failure in $strictFailures) {
        Write-Host "  - $failure" -ForegroundColor Red
    }

    throw "strict session proof failed with $($strictFailures.Count) issue(s): $($strictFailures -join '; ')"
}

Write-Host "STRICT RESULT: PASS" -ForegroundColor Green
