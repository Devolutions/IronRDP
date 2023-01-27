#!/bin/env pwsh

$ErrorActionPreference = "Stop"

Push-Location -Path $PSScriptRoot

try {
	cargo build

	if ($LastExitCode -ne 0) {
		throw "Compilation failed"
	}

	$undefinedSymbols = wasm2wat ./target/wasm32-unknown-unknown/debug/ironrdp.wasm | Select-String -Pattern 'import "env"'

	if (-Not [string]::IsNullOrEmpty($undefinedSymbols)) {
		throw "Found undefined symbols in generated wasm file: $undefinedSymbols"
	}

	Write-Host "Looks WASM-compatible to me!"
} finally {
	Pop-Location
}
