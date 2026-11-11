BeforeAll {
    $script:certificateScript = (Resolve-Path (Join-Path $PSScriptRoot '..\crates\ironrdp-idd\scripts\get-test-certificates.ps1')).Path
}

Describe 'Offline test signing certificate fixtures' {
    It 'uses the supplied CA and PFX without contacting the fixture URL' {
        # oracle: offline callers provide the exact CA and PFX files instead of downloading substitutes.
        $directory = Join-Path $TestDrive 'test-certificates'
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        $caPath = Join-Path $directory 'authenticode-test-ca.crt'
        $pfxPath = Join-Path $directory 'authenticode-test-cert.pfx'
        Set-Content -LiteralPath $caPath -Value 'fixture CA'

        $key = [Security.Cryptography.RSA]::Create(2048)
        $certificate = $null
        try {
            $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
                'CN=Offline Fixture',
                $key,
                [Security.Cryptography.HashAlgorithmName]::SHA256,
                [Security.Cryptography.RSASignaturePadding]::Pkcs1
            )
            $certificate = $request.CreateSelfSigned(
                [DateTimeOffset]::UtcNow.AddDays(-1),
                [DateTimeOffset]::UtcNow.AddDays(1)
            )
            [IO.File]::WriteAllBytes(
                $pfxPath,
                $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx, 'CodeSign123!')
            )
        } finally {
            if ($null -ne $certificate) { $certificate.Dispose() }
            $key.Dispose()
        }

        Mock Invoke-WebRequest { throw 'Offline fixture lookup attempted a network request' }
        $result = & $script:certificateScript -CertificateDirectory $directory

        $result.RootCertificate | Should -Be $caPath
        $result.Pfx | Should -Be $pfxPath
        Test-Path -LiteralPath $result.SignerCertificate -PathType Leaf | Should -BeTrue
        $signer = [Security.Cryptography.X509Certificates.X509Certificate2]::new($result.SignerCertificate)
        try {
            $signer.Subject | Should -Be 'CN=Offline Fixture'
        } finally {
            $signer.Dispose()
        }
        Should -Invoke Invoke-WebRequest -Times 0
    }

    It 'rejects an incomplete fixture directory without downloading substitutes' {
        # oracle: offline mode cannot silently fetch missing or unrelated signing material.
        $directory = Join-Path $TestDrive 'incomplete-certificates'
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $directory 'authenticode-test-ca.crt') -Value 'fixture CA'
        Mock Invoke-WebRequest { throw 'Offline fixture lookup attempted a network request' }

        { & $script:certificateScript -CertificateDirectory $directory } | Should -Throw
        Should -Invoke Invoke-WebRequest -Times 0
    }
}
