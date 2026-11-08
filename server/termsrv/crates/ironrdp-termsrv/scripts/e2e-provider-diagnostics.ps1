param([int]$Port, [string]$RemoteRoot = $(if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' }))
                        $providerProbe = $null
                        try {
                            $providerClsid = [guid]'{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}'
                            $providerType = [type]::GetTypeFromCLSID($providerClsid, $true)
                            $providerProbe = [System.Activator]::CreateInstance($providerType)
                            Write-Host 'Provider COM activation probe: success'
                        }
                        catch {
                            $hresult = if ($null -ne $_.Exception) { '0x{0:X8}' -f ($_.Exception.HResult -band 0xffffffff) } else { 'n/a' }
                            Write-Warning "Provider COM activation probe failed: $($_.Exception.Message) (hresult=$hresult)"
                        }
                        finally {
                            if ($null -ne $providerProbe -and [System.Runtime.InteropServices.Marshal]::IsComObject($providerProbe)) {
                                [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($providerProbe)
                            }
                        }
                            Write-Host '---- TermService / WUDFRd ----' -ForegroundColor Yellow
                            Get-Service -Name 'TermService', 'WUDFRd' -ErrorAction SilentlyContinue | Format-Table -AutoSize
                            Write-Host "---- listener check ($Port) ----" -ForegroundColor Yellow
                            Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue |
                                Select-Object LocalAddress, LocalPort, OwningProcess | Format-Table -AutoSize
                            Write-Host '---- qwinsta ----' -ForegroundColor Yellow
                            try {
                                qwinsta
                            }
                            catch {
                            }
                            Write-Host '---- ironrdp-termsrv.log (tail) ----' -ForegroundColor Yellow
                            $stdoutLog = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.log'
                            if (Test-Path -LiteralPath $stdoutLog -PathType Leaf) {
                                Get-Content -LiteralPath $stdoutLog -Tail 120 -ErrorAction SilentlyContinue
                            }
                            Write-Host '---- ironrdp-termsrv.err.log (tail) ----' -ForegroundColor Yellow
                            $stderrLog = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.err.log'
                            if (Test-Path -LiteralPath $stderrLog -PathType Leaf) {
                                Get-Content -LiteralPath $stderrLog -Tail 120 -ErrorAction SilentlyContinue
                            }
                            Write-Host '---- wts-provider-debug.log (tail) ----' -ForegroundColor Yellow
                            $providerLog = Join-Path $RemoteRoot 'logs\wts-provider-debug.log'
                            if (Test-Path -LiteralPath $providerLog -PathType Leaf) {
                                Get-Content -LiteralPath $providerLog -Tail 120 -ErrorAction SilentlyContinue
                            }
                            else {
                                Write-Host '<missing>'
                            }
                            Write-Host '---- idd runtime-config.txt ----' -ForegroundColor Yellow
                            Write-Host '---- wts-provider-debug fallback log (tail) ----' -ForegroundColor Yellow
                            if (Test-Path -LiteralPath 'C:\Windows\Temp\wts-provider-debug.log' -PathType Leaf) {
                                Get-Content -LiteralPath 'C:\Windows\Temp\wts-provider-debug.log' -Tail 120 -ErrorAction SilentlyContinue
                            }
                            else {
                                Write-Host '<missing>'
                            }
                            Write-Host '---- TermService Environment ----' -ForegroundColor Yellow
                            try {
                                Get-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Services\TermService' -Name 'Environment' -ErrorAction Stop |
                                    Select-Object -ExpandProperty Environment
                            }
                            catch {
                                Write-Host '<missing>'
                            }
                            Write-Host '---- WinStations registry ----' -ForegroundColor Yellow
                            Get-ChildItem -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations' -ErrorAction SilentlyContinue |
                                ForEach-Object {
                                    $props = Get-ItemProperty -Path $_.PSPath -ErrorAction SilentlyContinue
                                    $portNumber = $null
                                    $loadableProtocolObject = $null
                                    $pdDll = $null
                                    if ($null -ne $props) {
                                        $portProperty = $props.PSObject.Properties['PortNumber']
                                        if ($null -ne $portProperty) { $portNumber = $portProperty.Value }
                                        $loadableProtocolProperty = $props.PSObject.Properties['LoadableProtocol_Object']
                                        if ($null -ne $loadableProtocolProperty) { $loadableProtocolObject = $loadableProtocolProperty.Value }
                                        $pdDllProperty = $props.PSObject.Properties['PdDLL']
                                        if ($null -ne $pdDllProperty) { $pdDll = $pdDllProperty.Value }
                                    }
                                    [pscustomobject]@{
                                        Name = $_.PSChildName
                                        PortNumber = $portNumber
                                        LoadableProtocolObject = $loadableProtocolObject
                                        PdDll = $pdDll
                                    }
                                } | Format-Table -AutoSize
                            Write-Host '---- IRDP-Tcp registry ----' -ForegroundColor Yellow
                            if (Test-Path -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp') {
                                Get-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp' -ErrorAction SilentlyContinue |
                                    Select-Object PortNumber, LoadableProtocol_Object, PdDLL, PdName, PdClass, fEnableWinStation | Format-List
                            }
                            else {
                                Write-Host '<missing>'
                            }
                            Write-Host '---- Terminal Services events ----' -ForegroundColor Yellow
                            foreach ($logName in @(
                                'Microsoft-Windows-TerminalServices-LocalSessionManager/Operational',
                                'Microsoft-Windows-RemoteDesktopServices-RdpCoreTS/Operational'
                            )) {
                                Write-Host "[$logName]" -ForegroundColor DarkYellow
                                try {
                                    Get-WinEvent -LogName $logName -MaxEvents 8 -ErrorAction Stop |
                                        Select-Object TimeCreated, Id, LevelDisplayName, Message | Format-List
                                }
                                catch {
                                    Write-Host '<unavailable>'
                                }
                            Write-Host '---- COM/Application/System events ----' -ForegroundColor Yellow
                            foreach ($logName in @(
                                'Microsoft-Windows-COMRuntime/Operational',
                                'Application',
                                'System'
                            )) {
                                Write-Host "[$logName]" -ForegroundColor DarkYellow
                                try {
                                    $events = Get-WinEvent -LogName $logName -MaxEvents 40 -ErrorAction Stop |
                                        Where-Object {
                                            $message = $_.Message
                                            if ([string]::IsNullOrWhiteSpace($message)) { return $false }
                                            $message -match 'ironrdp|wtsprotocol|IRDP-Tcp|89C7ED1E|TermService|svchost|LoadableProtocol_Object|CLSID'
                                        } |
                                        Select-Object -First 8 TimeCreated, Id, ProviderName, LevelDisplayName, Message
                                    if ($null -eq $events -or @($events).Count -eq 0) {
                                        Write-Host '<no matching events>'
                                    }
                                    else {
                                        $events | Format-List
                                    }
                                }
                                catch {
                                    Write-Host '<unavailable>'
                                }
                            }
                            }
                            if (Test-Path -LiteralPath 'C:\ProgramData\IronRDP\Idd\runtime-config.txt' -PathType Leaf) {
                                Get-Content -LiteralPath 'C:\ProgramData\IronRDP\Idd\runtime-config.txt' -ErrorAction SilentlyContinue
                            }
                            else {
                                Write-Host '<missing>'
                            }
                            Write-Host '---- idd debug trace (tail) ----' -ForegroundColor Yellow
                            if (Test-Path -LiteralPath 'C:\ProgramData\IronRDP\Idd\ironrdp-idd-debug.log' -PathType Leaf) {
                                Get-Content -LiteralPath 'C:\ProgramData\IronRDP\Idd\ironrdp-idd-debug.log' -Tail 120 -ErrorAction SilentlyContinue
                            }
                            else {
                                Write-Host '<missing>'
                            }
                            Write-Host '---- IronRDP PnP devices ----' -ForegroundColor Yellow
                            Get-PnpDevice -FriendlyName 'IronRDP*' -ErrorAction SilentlyContinue |
                                Select-Object Status, Class, FriendlyName, InstanceId | Format-Table -AutoSize
