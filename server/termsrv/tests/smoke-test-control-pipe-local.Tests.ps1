BeforeAll {
    $scriptPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-control-pipe-local.ps1'
    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) { throw $parseErrors[0] }

    $functionAst = $ast.Find({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -eq 'Read-ExactStreamBytes'
    }, $true)
    if ($null -eq $functionAst) { throw 'Read-ExactStreamBytes was not found' }
    Invoke-Expression $functionAst.Extent.Text

    $streamTypeName = 'ChunkedReadStream' + [guid]::NewGuid().ToString('N')
    Add-Type -TypeDefinition @"
using System;
using System.IO;

namespace IronRdpTest
{
    public sealed class $streamTypeName : Stream
    {
        private readonly byte[] data;
        private readonly int maxRead;
        private int position;

        public $streamTypeName(byte[] data, int maxRead)
        {
            this.data = data;
            this.maxRead = maxRead;
        }

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => throw new NotSupportedException();
        public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }
        public override void Flush() => throw new NotSupportedException();
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        public override int Read(byte[] buffer, int offset, int count)
        {
            var available = data.Length - position;
            var read = Math.Min(Math.Min(count, maxRead), available);
            Array.Copy(data, position, buffer, offset, read);
            position += read;
            return read;
        }
    }
}
"@
    $script:ChunkedReadStreamType = "IronRdpTest.$streamTypeName"
}

Describe 'Read-ExactStreamBytes' {
    It 'reads the complete response when the stream returns partial chunks' {
        # oracle: mutant partial stream reads are returned without assembling the requested response bytes.
        $bytes = [byte[]](1, 2, 3, 4, 5)
        $stream = [Activator]::CreateInstance([type]$script:ChunkedReadStreamType, [object[]]@($bytes, 2))

        $result = Read-ExactStreamBytes -Stream $stream -Count 5

        [BitConverter]::ToString($result) | Should -BeExactly '01-02-03-04-05'
    }

    It 'fails when the stream closes before the response is complete' {
        # oracle: mutant a closed response stream is accepted as a complete control message.
        $bytes = [byte[]](1, 2)
        $stream = [Activator]::CreateInstance([type]$script:ChunkedReadStreamType, [object[]]@($bytes, 1))

        { Read-ExactStreamBytes -Stream $stream -Count 4 } | Should -Throw '*control pipe closed while reading response*'
    }
}
