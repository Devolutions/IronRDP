#:package Grpc.Net.Client@2.71.0
#:package Grpc.Core.Api@2.71.0
#:package Google.Protobuf@3.30.2

// Thin Windows Sandbox gRPC client for ironrdp-agent.
// Speaks sandboxserver.SandboxCore over \\.\pipe\wsandbox\<md5(user SID)>.
//
// Usage:
//   dotnet run windows_sandbox_grpc.cs -- list
//   dotnet run windows_sandbox_grpc.cs -- config <sandbox-id>
//   dotnet run windows_sandbox_grpc.cs -- stop <sandbox-id>
//
// config prints one JSON object on stdout (password included — treat as secret).

using System.IO.Pipes;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using System.Xml.Linq;
using Google.Protobuf;
using Grpc.Core;
using Grpc.Net.Client;

static string GetPipeName()
{
    var sid = WindowsIdentity.GetCurrent().User!.Value;
    var hash = MD5.HashData(Encoding.UTF8.GetBytes(sid));
    return Path.Combine("wsandbox", new Guid(hash).ToString());
}

static async ValueTask<Stream> ConnectGrpcPipe(SocketsHttpConnectionContext _, CancellationToken ct)
{
    var client = new NamedPipeClientStream(
        ".",
        GetPipeName(),
        PipeDirection.InOut,
        PipeOptions.WriteThrough | PipeOptions.Asynchronous,
        TokenImpersonationLevel.Anonymous);
    await client.ConnectAsync(TimeSpan.FromSeconds(15), ct);
    return client;
}

static byte[] EncodeStringField(int fieldNumber, string value)
{
    using var ms = new MemoryStream();
    var cos = new CodedOutputStream(ms);
    cos.WriteTag((uint)((fieldNumber << 3) | 2));
    cos.WriteString(value);
    cos.Flush();
    return ms.ToArray();
}

static byte[] EncodeEmpty() => Array.Empty<byte>();

static (int hr, string cfg, List<string> ids) ParseReply(byte[] data, bool expectIds)
{
    var input = new CodedInputStream(data);
    int hr = 0;
    string cfg = "";
    var ids = new List<string>();
    uint tag;
    while ((tag = input.ReadTag()) != 0)
    {
        // hresult is sfixed32 field 1 → wire tag (1<<3)|5 = 13
        if (tag == 13)
        {
            hr = input.ReadSFixed32();
        }
        // rdp_client_config string field 2 → tag 18
        else if (tag == 18 && !expectIds)
        {
            cfg = input.ReadString();
        }
        // sandbox_ids repeated string field 2 → tag 18
        else if (tag == 18 && expectIds)
        {
            ids.Add(input.ReadString());
        }
        else
        {
            input.SkipLastField();
        }
    }
    return (hr, cfg, ids);
}

static string JsonEscape(string? s)
{
    s ??= "";
    return s
        .Replace("\\", "\\\\", StringComparison.Ordinal)
        .Replace("\"", "\\\"", StringComparison.Ordinal)
        .Replace("\r", "\\r", StringComparison.Ordinal)
        .Replace("\n", "\\n", StringComparison.Ordinal)
        .Replace("\t", "\\t", StringComparison.Ordinal);
}

static string ParseConfigXmlToJson(string cfgXml, string sandboxId)
{
    var doc = XDocument.Parse(cfgXml);
    string Local(string name) =>
        doc.Descendants().FirstOrDefault(e => e.Name.LocalName == name)?.Value ?? "";

    var vmId = Local("VMId").Trim('{', '}');
    var transport = Local("RdpTransport");
    var username = Local("Username");
    var password = Local("Password");
    var ip = Local("IpAddress");
    var sid = Local("SandboxId").Trim('{', '}');
    if (string.IsNullOrEmpty(sid))
        sid = sandboxId;

    var pipePath = string.IsNullOrEmpty(vmId) ? "" : $@"\\.\pipe\{vmId}";
    var clip = bool.TryParse(Local("ClipboardRedirection"), out var c) && c;
    var smart = bool.TryParse(Local("SmartCardRedirection"), out var s) && s;

    // Hand-written JSON: file-based `dotnet run` disables reflection JsonSerializer by default.
    return
        "{"
        + $"\"sandbox_id\":\"{JsonEscape(sid)}\","
        + $"\"vm_id\":\"{JsonEscape(vmId)}\","
        + $"\"username\":\"{JsonEscape(username)}\","
        + $"\"password\":\"{JsonEscape(password)}\","
        + $"\"rdp_transport\":\"{JsonEscape(transport)}\","
        + $"\"ip_address\":\"{JsonEscape(ip)}\","
        + $"\"pipe_path\":{(string.IsNullOrEmpty(pipePath) ? "null" : $"\"{JsonEscape(pipePath)}\"")},"
        + $"\"clipboard_redirection\":{(clip ? "true" : "false")},"
        + $"\"smartcard_redirection\":{(smart ? "true" : "false")}"
        + "}";
}

static string JsonStringArray(string propertyName, IEnumerable<string> values)
{
    var items = string.Join(",", values.Select(v => $"\"{JsonEscape(v)}\""));
    return $"{{\"{propertyName}\":[{items}]}}";
}

if (args.Length == 0)
{
    Console.Error.WriteLine("usage: list | config <sandbox-id> | stop <sandbox-id>");
    return 2;
}

var handler = new SocketsHttpHandler
{
    ConnectCallback = ConnectGrpcPipe,
    ConnectTimeout = TimeSpan.FromSeconds(15),
};
using var channel = GrpcChannel.ForAddress("http://localhost", new GrpcChannelOptions { HttpHandler = handler });
var invoker = channel.CreateCallInvoker();

var cmd = args[0].ToLowerInvariant();
try
{
    switch (cmd)
    {
        case "list":
        {
            var method = new Method<byte[], byte[]>(
                MethodType.Unary,
                "sandboxserver.SandboxCore",
                "EnumerateSandboxVMs",
                Marshallers.Create<byte[]>(b => b, b => b),
                Marshallers.Create<byte[]>(b => b, b => b));
            var resp = await invoker.AsyncUnaryCall(method, null, default, EncodeEmpty());
            var (hr, _, ids) = ParseReply(resp, expectIds: true);
            if (hr != 0)
            {
                Console.Error.WriteLine($"EnumerateSandboxVMs hresult=0x{hr:X8}");
                return 1;
            }
            Console.WriteLine(JsonStringArray("sandbox_ids", ids));
            return 0;
        }
        case "config":
        {
            if (args.Length < 2)
            {
                Console.Error.WriteLine("config requires <sandbox-id>");
                return 2;
            }
            var sandboxId = args[1];
            var method = new Method<byte[], byte[]>(
                MethodType.Unary,
                "sandboxserver.SandboxCore",
                "GetRdpClientConfig",
                Marshallers.Create<byte[]>(b => b, b => b),
                Marshallers.Create<byte[]>(b => b, b => b));
            var resp = await invoker.AsyncUnaryCall(method, null, default, EncodeStringField(1, sandboxId));
            var (hr, cfg, _) = ParseReply(resp, expectIds: false);
            if (hr != 0)
            {
                Console.Error.WriteLine($"GetRdpClientConfig hresult=0x{hr:X8}");
                return 1;
            }
            if (string.IsNullOrEmpty(cfg))
            {
                Console.Error.WriteLine("empty RdpClientConfig");
                return 1;
            }
                        Console.WriteLine(ParseConfigXmlToJson(cfg, sandboxId));
            return 0;
        }
        case "stop":
        {
            if (args.Length < 2)
            {
                Console.Error.WriteLine("stop requires <sandbox-id>");
                return 2;
            }
            var method = new Method<byte[], byte[]>(
                MethodType.Unary,
                "sandboxserver.SandboxCore",
                "ShutdownSandbox",
                Marshallers.Create<byte[]>(b => b, b => b),
                Marshallers.Create<byte[]>(b => b, b => b));
            var resp = await invoker.AsyncUnaryCall(method, null, default, EncodeStringField(1, args[1]));
            var (hr, _, _) = ParseReply(resp, expectIds: false);
            if (hr != 0)
            {
                Console.Error.WriteLine($"ShutdownSandbox hresult=0x{hr:X8}");
                return 1;
            }
                        Console.WriteLine($"{{\"ok\":true,\"sandbox_id\":\"{JsonEscape(args[1])}\"}}");
            return 0;
        }
        default:
            Console.Error.WriteLine($"unknown command '{cmd}'");
            return 2;
    }
}
catch (Exception ex)
{
    Console.Error.WriteLine(ex.Message);
    return 1;
}
