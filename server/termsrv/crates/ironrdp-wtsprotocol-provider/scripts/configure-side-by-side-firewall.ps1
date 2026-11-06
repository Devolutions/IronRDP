[CmdletBinding()]
param(
    [Parameter()]
    [ValidateSet("Add", "Remove", "Verify")]
    [string]$Mode = "Verify",

    [Parameter()]
    [ValidateRange(0, 65535)]
    [int]$PortNumber = 0,

    [Parameter()]
    [string]$RuleName = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-Rule {
    return Get-NetFirewallRule -DisplayName $resolvedRuleName -ErrorAction SilentlyContinue
}

$resolvedRuleName = if ([string]::IsNullOrWhiteSpace($RuleName)) {
    "IronRDP Side-by-side RDP (TCP $PortNumber)"
} else {
    $RuleName
}

if ($Mode -in @("Add", "Remove") -and -not (Test-IsAdministrator)) {
    throw "this script must be run from an elevated PowerShell session for Mode=$Mode"
}

if ($Mode -eq "Add") {
    $existing = Get-Rule
    if ($existing) {
        $existing | Remove-NetFirewallRule
    }

    New-NetFirewallRule `
        -DisplayName $resolvedRuleName `
        -Direction Inbound `
        -Profile Any `
        -Action Allow `
        -Enabled True `
        -Protocol TCP `
        -LocalPort $PortNumber | Out-Null

    Write-Host "Firewall rule added"
    Write-Host "  name: $resolvedRuleName"
    Write-Host "  tcp port: $PortNumber"
    return
}

if ($Mode -eq "Remove") {
    $existing = Get-Rule
    if ($existing) {
        $existing | Remove-NetFirewallRule
        Write-Host "Firewall rule removed: $resolvedRuleName"
    } else {
        Write-Host "Firewall rule not present: $resolvedRuleName"
    }

    return
}

$rule = Get-Rule
if (-not $rule) {
    throw "firewall rule not found: $resolvedRuleName"
}

$portFilter = $rule | Get-NetFirewallPortFilter
if (-not ($portFilter.LocalPort -contains "$PortNumber")) {
    throw "firewall rule port mismatch: expected $PortNumber got $($portFilter.LocalPort -join ',')"
}

if (-not ($rule.Enabled -contains "True")) {
    throw "firewall rule is disabled: $resolvedRuleName"
}

Write-Host "Firewall rule verification passed"
Write-Host "  name: $resolvedRuleName"
Write-Host "  tcp port: $PortNumber"
