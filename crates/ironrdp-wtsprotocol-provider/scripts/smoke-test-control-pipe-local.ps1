[CmdletBinding()]
param(
    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$PipeName = "IronRdpWtsControl",

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ListenerName = "IRDP-Tcp",

    [Parameter()]
    [ValidateRange(1, 65535)]
    [int]$PortNumber = 4496,

    [Parameter()]
    [ValidateRange(1, 10000)]
    [int]$IncomingTimeoutMs = 2000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-ControlCommand {
    param(
        [Parameter(Mandatory = $true)]
        [hashtable]$Command,

        [Parameter(Mandatory = $true)]
        [string]$Pipe
    )

    $client = New-Object System.IO.Pipes.NamedPipeClientStream(
        '.',
        $Pipe,
        [System.IO.Pipes.PipeDirection]::InOut,
        [System.IO.Pipes.PipeOptions]::None
    )

    $client.Connect(5000)

    try {
        $json = $Command | ConvertTo-Json -Compress
        $payload = [System.Text.Encoding]::UTF8.GetBytes($json)

        $payloadLengthBytes = [BitConverter]::GetBytes([uint32]$payload.Length)
        $client.Write($payloadLengthBytes, 0, $payloadLengthBytes.Length)
        $client.Write($payload, 0, $payload.Length)
        $client.Flush()

        $responseLengthBytes = New-Object byte[] 4
        [void]$client.Read($responseLengthBytes, 0, 4)
        $responseLength = [BitConverter]::ToUInt32($responseLengthBytes, 0)

        $responseBytes = New-Object byte[] $responseLength
        $offset = 0

        while ($offset -lt $responseLength) {
            $read = $client.Read($responseBytes, $offset, $responseLength - $offset)
            if ($read -le 0) {
                throw "control pipe closed while reading response"
            }

            $offset += $read
        }

        $responseJson = [System.Text.Encoding]::UTF8.GetString($responseBytes)
        return ($responseJson | ConvertFrom-Json)
    }
    finally {
        $client.Dispose()
    }
}

$results = New-Object System.Collections.Generic.List[object]
$tcpClient = $null

try {
    $results.Add((Invoke-ControlCommand -Pipe $PipeName -Command @{
                type = "start_listen"
                listener_name = $ListenerName
            }))

    $results.Add((Invoke-ControlCommand -Pipe $PipeName -Command @{
                type = "wait_for_incoming"
                listener_name = $ListenerName
                timeout_ms = 10
            }))

    $tcpClient = [System.Net.Sockets.TcpClient]::new()
    $tcpClient.Connect('127.0.0.1', $PortNumber)

    $incoming = Invoke-ControlCommand -Pipe $PipeName -Command @{
        type = "wait_for_incoming"
        listener_name = $ListenerName
        timeout_ms = $IncomingTimeoutMs
    }

    $results.Add($incoming)

    if ($incoming.type -eq "incoming_connection") {
        $connectionId = [uint32]$incoming.connection_id

        $results.Add((Invoke-ControlCommand -Pipe $PipeName -Command @{
                    type = "accept_connection"
                    connection_id = $connectionId
                }))

        $results.Add((Invoke-ControlCommand -Pipe $PipeName -Command @{
                    type = "close_connection"
                    connection_id = $connectionId
                }))
    }

    $results.Add((Invoke-ControlCommand -Pipe $PipeName -Command @{
                type = "stop_listen"
                listener_name = $ListenerName
            }))
}
finally {
    if ($null -ne $tcpClient) {
        $tcpClient.Dispose()
    }
}

$results | ConvertTo-Json -Depth 5 | Write-Host
