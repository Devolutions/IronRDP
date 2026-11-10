Set-StrictMode -Version Latest

$script:SideBySideDefaultListenerPort = 4489
$script:SideBySidePortRegistryPath = "HKLM:\SOFTWARE\IronRDP\WtsProtocolProvider"
$script:SideBySidePortRegistryValueName = "ListenerPort"

function Get-SideBySideListenerPortSettingInfo {
    return [PSCustomObject]@{
        DefaultPort = [int]$script:SideBySideDefaultListenerPort
        RegistryPath = $script:SideBySidePortRegistryPath
        RegistryValueName = $script:SideBySidePortRegistryValueName
    }
}

function Get-SideBySideDefaultListenerPort {
    $defaultPort = [int]$script:SideBySideDefaultListenerPort

    if (-not (Test-Path -LiteralPath $script:SideBySidePortRegistryPath -PathType Container)) {
        return $defaultPort
    }

    try {
        $properties = Get-ItemProperty -LiteralPath $script:SideBySidePortRegistryPath -ErrorAction Stop
    } catch {
        Write-Warning "failed to read listener port registry key $($script:SideBySidePortRegistryPath): $($_.Exception.Message)"
        return $defaultPort
    }

    $valueProperty = $properties.PSObject.Properties[$script:SideBySidePortRegistryValueName]
    if ($null -eq $valueProperty) {
        return $defaultPort
    }

    try {
        $candidate = [int]$valueProperty.Value
    } catch {
        Write-Warning "listener port registry value $($script:SideBySidePortRegistryPath)\\$($script:SideBySidePortRegistryValueName) is not a valid integer; using default port $defaultPort"
        return $defaultPort
    }

    if ($candidate -lt 1 -or $candidate -gt 65535) {
        Write-Warning "listener port registry value $($script:SideBySidePortRegistryPath)\\$($script:SideBySidePortRegistryValueName) is outside valid range (1-65535); using default port $defaultPort"
        return $defaultPort
    }

    return $candidate
}

function Set-SideBySideDefaultListenerPort {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateRange(1, 65535)]
        [int]$PortNumber
    )

    New-Item -Path $script:SideBySidePortRegistryPath -ItemType Directory -Force | Out-Null
    New-ItemProperty -Path $script:SideBySidePortRegistryPath -Name $script:SideBySidePortRegistryValueName -PropertyType DWord -Value $PortNumber -Force | Out-Null

    return $PortNumber
}

function Resolve-SideBySideListenerPort {
    param(
        [Parameter()]
        [ValidateRange(0, 65535)]
        [int]$PortNumber = 0,

        [Parameter()]
        [switch]$PersistResolvedDefault
    )

    if ($PortNumber -gt 0) {
        return $PortNumber
    }

    $resolvedPort = Get-SideBySideDefaultListenerPort
    if ($PersistResolvedDefault.IsPresent) {
        Set-SideBySideDefaultListenerPort -PortNumber $resolvedPort | Out-Null
    }

    return $resolvedPort
}