BeforeAll {
    $verdictScript = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\assert-e2e-session-proof.ps1'
    . (Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\e2e-test-screenshot.helpers.ps1')

    function New-CompleteProviderEvidence {
        return @{
            Mode = 'Provider'; evidenceCorrelated = $true; evidenceCorrelationReason = ''
            RdpUsername = 'fixture'; securityLogonType10Count = 1; termsrvFallbackMarkerCount = 0
            providerSessionProofMarkerCount = 1; notifyCommandProcessCreatedCount = 1; termsrvSessionProofMarkerCount = 1
            guiTargetSessionResolved = $true; guiTargetSessionProcessProof = $true; guiTargetSessionId = 4
            guiTargetSessionSource = 'fixture'; guiTargetSessionExplorerCount = 1; guiTargetSessionGuiProcessCount = 1
            guiTargetSessionWinlogonCount = 1; guiTargetSessionLogonUiCount = 0
            providerHardwareIdCustom = $true; providerGetHardwareIdCount = 1; providerOnDriverLoadCount = 1
            providerCustomVideoSourceSelected = $true; providerVideoHandleDriverLoadCount = 1
            providerManualVideoSourceSelected = $false; providerFallbackVideoSourceSelected = $false; providerAssumePresentCount = 0
            iddMonitorArrivalMarkerCount = 1; iddDisplayConfigUpdateRequestCount = 1; iddDisplayConfigUpdateSuccessCount = 1
            iddCommitModesMarkerCount = 1; iddCommitModesActivePathCount = 1; iddSwapchainAssignedCount = 1
            iddSessionReadyMarkerCount = 1; iddFirstFrameMarkerCount = 1; iddDumpCount = 1
            iddTargetSessionHasGraphics = $true; type10IddGraphicsSessionConfirmed = $true
            providerVideoHandleMarkerCount = 1; termsrvIddMarkerCount = 1; providerLogonPolicyMarkerCount = 1
            iddDriverLoadedNotified = $true; iddWddmEnabledSignalCount = 1; iddObservedSessionIds = @(4)
            iddTargetSessionMatchCount = 1
        }
    }

    function New-CompleteProviderVerdictEvidence {
        $result = @{
            screenshotExists = $true; screenshotFileInfo = @{ Length = 2000 }
            analysisSucceeded = $true; hasMeaningfulContent = $true; remoteLogCollectionSucceeded = $true
            preflightBlockedConnectionAttempt = $false; hardPreflightIssues = @()
        }
        $provider = New-CompleteProviderEvidence
        foreach ($key in $provider.Keys) {
            $result[$key] = $provider[$key]
        }
        return $result
    }
}

Describe 'E2E verdict' {
    BeforeEach {
        $evidence = @{
            Mode = 'Standalone'
            preflightBlockedConnectionAttempt = $false
            hardPreflightIssues = @()
            screenshotExists = $true
            screenshotFileInfo = @{ Length = 2000 }
            analysisSucceeded = $true
            hasMeaningfulContent = $true
            remoteLogCollectionSucceeded = $true
        }
    }

    It 'rejects missing remote evidence despite a good screenshot' {
        # oracle: server-side evidence is mandatory and a client screenshot is not equivalent.
        $evidence.remoteLogCollectionSucceeded = $false
        { & $verdictScript -Evidence $evidence } | Should -Throw '*strict session proof failed*'
    }

    It 'rejects a blank frame without strict mode' {
        # oracle: blank or uniform images are failed graphics observations.
        $evidence.hasMeaningfulContent = $false
        { Assert-E2EOutput -Evidence $evidence -SkipScreenshot $false } | Should -Throw '*blank or uniform*'
    }

    It 'allows explicitly skipped screenshot collection without requiring an image' {
        # oracle: diagnostic-only collection must remain independently callable.
        $evidence.screenshotExists = $false
        { Assert-E2EOutput -Evidence $evidence -SkipScreenshot $true } | Should -Not -Throw
    }

    It 'accepts meaningful standalone output and successful evidence collection' {
        # oracle: standalone success requires current graphics and server evidence, not provider-only callbacks.
        { & $verdictScript -Evidence $evidence } | Should -Not -Throw
    }

    It 'does not accept otherwise complete provider proof without current-connection correlation' {
        # oracle: mutant strict proof combines unrelated logon and graphics signals.
        $provider = New-CompleteProviderEvidence
        $provider.evidenceCorrelationReason = 'fixture'
        foreach ($key in $provider.Keys) { $evidence[$key] = $provider[$key] }
        { & $verdictScript -Evidence $evidence } | Should -Not -Throw
        $evidence.evidenceCorrelated = $false
        { & $verdictScript -Evidence $evidence } | Should -Throw '*connection evidence is not correlated*'
    }

    It 'requires correlated Provider logon, session transition, and GUI-session evidence' {
        # oracle: mutant strict verdict accepts a provider session with a missing protocol proof edge.
        $cases = @(
            @{ Key = 'securityLogonType10Count'; Value = 0; Message = 'mandatory logon proof missing' }
            @{ Key = 'termsrvFallbackMarkerCount'; Value = 1; Message = 'termsrv fallback markers detected' }
            @{ Key = 'providerSessionProofMarkerCount'; Value = 0; Message = 'provider session proof markers were not observed' }
            @{ Key = 'notifyCommandProcessCreatedCount'; Value = 0; Message = 'shell transition marker missing' }
            @{ Key = 'termsrvSessionProofMarkerCount'; Value = 0; Message = 'termsrv session proof markers were not observed' }
            @{ Key = 'guiTargetSessionResolved'; Value = $false; Message = 'GUI session gate missing' }
            @{ Key = 'guiTargetSessionProcessProof'; Value = $false; Message = 'GUI session gate failed' }
        )
        foreach ($case in $cases) {
            $provider = New-CompleteProviderEvidence
            $provider[$case.Key] = $case.Value
            $providerEvidence = New-CompleteProviderVerdictEvidence
            foreach ($key in $provider.Keys) { $providerEvidence[$key] = $provider[$key] }
            { & $verdictScript -Evidence $providerEvidence } | Should -Throw "*$($case.Message)*"
        }
    }

    It 'requires every custom IDD, selected video-handle, and target-session graphics proof' {
        # oracle: mutant provider verdict accepts synthetic, incomplete, or cross-session graphics evidence.
        $cases = @(
            @{ Key = 'providerHardwareIdCustom'; Value = $false; Message = 'custom IDD hardware id was not confirmed' }
            @{ Key = 'providerGetHardwareIdCount'; Value = 0; Message = 'custom IDD hardware id callback was not observed' }
            @{ Key = 'providerOnDriverLoadCount'; Value = 0; Message = 'OnDriverLoad(session, nonzero_handle) was not observed' }
            @{ Key = 'providerCustomVideoSourceSelected'; Value = $false; Message = 'provider did not return the OnDriverLoad handle' }
            @{ Key = 'providerVideoHandleDriverLoadCount'; Value = 0; Message = 'provider did not log a GetVideoHandle selection' }
            @{ Key = 'providerManualVideoSourceSelected'; Value = $true; Message = 'provider selected a manual custom-device video source' }
            @{ Key = 'providerFallbackVideoSourceSelected'; Value = $true; Message = 'provider fell back to rdp_video_miniport' }
            @{ Key = 'providerAssumePresentCount'; Value = 1; Message = 'provider emitted synthetic ASSUME_PRESENT markers' }
            @{ Key = 'iddMonitorArrivalMarkerCount'; Value = 0; Message = 'IDD monitor-arrival marker was not observed' }
            @{ Key = 'iddDisplayConfigUpdateRequestCount'; Value = 0; Message = 'IddCxDisplayConfigUpdate request marker was not observed' }
            @{ Key = 'iddDisplayConfigUpdateSuccessCount'; Value = 0; Message = 'IddCxDisplayConfigUpdate success marker was not observed' }
            @{ Key = 'iddCommitModesMarkerCount'; Value = 0; Message = 'CommitModes marker was not observed' }
            @{ Key = 'iddCommitModesActivePathCount'; Value = 0; Message = 'CommitModes never reported an active display path' }
            @{ Key = 'iddSwapchainAssignedCount'; Value = 0; Message = 'swapchain assignment marker was not observed' }
            @{ Key = 'iddSessionReadyMarkerCount'; Value = 0; Message = 'IDD session-ready marker was not observed' }
            @{ Key = 'iddFirstFrameMarkerCount'; Value = 0; Message = 'IDD first-frame marker was not observed' }
            @{ Key = 'iddDumpCount'; Value = 0; Message = 'no custom IDD bitmap dumps were downloaded' }
            @{ Key = 'iddTargetSessionHasGraphics'; Value = $false; Message = 'graphics-session proof missing' }
            @{ Key = 'type10IddGraphicsSessionConfirmed'; Value = $false; Message = 'mandatory type10+custom-idd-graphics proof missing' }
            @{ Key = 'providerVideoHandleMarkerCount'; Value = $null; Message = 'IDD provider video-handle marker count was not collected' }
            @{ Key = 'termsrvIddMarkerCount'; Value = $null; Message = 'IDD termsrv marker count was not collected' }
            @{ Key = 'providerLogonPolicyMarkerCount'; Value = $null; Message = 'provider logon-policy marker count was not collected' }
        )
        foreach ($case in $cases) {
            $provider = New-CompleteProviderEvidence
            $provider[$case.Key] = $case.Value
            $providerEvidence = New-CompleteProviderVerdictEvidence
            foreach ($key in $provider.Keys) { $providerEvidence[$key] = $provider[$key] }
            { & $verdictScript -Evidence $providerEvidence } | Should -Throw "*$($case.Message)*"
        }

        $providerEvidence = New-CompleteProviderVerdictEvidence
        $providerEvidence.providerOnDriverLoadCount = 0
        $providerEvidence.iddDriverLoadedNotified = $false
        $providerEvidence.iddWddmEnabledSignalCount = 0
        $providerEvidence.providerVideoHandleMarkerCount = 0
        $providerEvidence.termsrvIddMarkerCount = 0
        $providerEvidence.iddDisplayConfigUpdateRequestCount = 0
        $providerEvidence.iddCommitModesMarkerCount = 0
        $providerEvidence.iddSwapchainAssignedCount = 0
        { & $verdictScript -Evidence $providerEvidence } | Should -Throw '*IDD diagnostics gate missing*'
    }
}
