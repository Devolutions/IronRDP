using System;
using System.Net.Http;
using System.Net.Http.Json;
using System.Text.Json.Serialization;
using System.Threading.Tasks;

namespace Devolutions.IronRdp.AvaloniaExample;

/// <summary>
/// Client for requesting JWT tokens from a Devolutions Gateway tokengen server.
/// </summary>
public class TokenGenerator : IDisposable
{
    private readonly HttpClient _client;
    private readonly string _tokengenUrl;

    /// <summary>
    /// Creates a new TokenGenerator instance.
    /// </summary>
    /// <param name="tokengenUrl">The base URL of the tokengen server (e.g., "http://localhost:8080")</param>
    public TokenGenerator(string tokengenUrl = "http://localhost:8080")
    {
        _tokengenUrl = tokengenUrl;
        _client = new HttpClient
        {
            Timeout = TimeSpan.FromSeconds(30)
        };
    }

    /// <summary>
    /// Generates an RDP token with credential injection for gateway-based connections.
    /// </summary>
    /// <param name="dstHost">Destination RDP server (e.g., "10.10.0.3:3389")</param>
    /// <param name="proxyUser">Gateway proxy username</param>
    /// <param name="proxyPassword">Gateway proxy password</param>
    /// <param name="destUser">Destination RDP server username</param>
    /// <param name="destPassword">Destination RDP server password</param>
    /// <param name="jetAid">Optional session UUID</param>
    /// <param name="validityDuration">Token validity in seconds (default: 3600)</param>
    /// <returns>A JWT token string</returns>
    public async Task<string> GenerateRdpTlsToken(
        string dstHost,
        string proxyUser,
        string proxyPassword,
        string destUser,
        string destPassword,
        string? jetAid = null,
        int validityDuration = 3600)
    {
        var request = new RdpTlsTokenRequest
        {
            DstHst = dstHost,
            PrxUsr = proxyUser,
            PrxPwd = proxyPassword,
            DstUsr = destUser,
            DstPwd = destPassword,
            JetAid = jetAid,
            ValidityDuration = validityDuration
        };

        try
        {
            var response = await _client.PostAsJsonAsync($"{_tokengenUrl}/rdp_tls", request);
            response.EnsureSuccessStatusCode();

            var result = await response.Content.ReadFromJsonAsync<TokenResponse>();
            if (result?.Token == null)
            {
                throw new Exception("Token generation failed: Empty response");
            }

            return result.Token;
        }
        catch (HttpRequestException ex)
        {
            throw new Exception($"Failed to connect to tokengen server at {_tokengenUrl}: {ex.Message}", ex);
        }
        catch (TaskCanceledException ex)
        {
            throw new Exception($"Token generation request timed out: {ex.Message}", ex);
        }
    }

    /// <summary>
    /// Generates a forward mode token for simple RDP forwarding without credential injection.
    /// </summary>
    /// <param name="dstHost">Destination host</param>
    /// <param name="jetAp">Application protocol (default: "rdp")</param>
    /// <param name="jetRec">Enable recording</param>
    /// <param name="validityDuration">Token validity in seconds (default: 3600)</param>
    /// <returns>A JWT token string</returns>
    public async Task<string> GenerateForwardToken(
        string dstHost,
        string jetAp = "rdp",
        bool jetRec = false,
        int validityDuration = 3600)
    {
        var request = new ForwardTokenRequest
        {
            DstHst = dstHost,
            JetAp = jetAp,
            JetRec = jetRec,
            ValidityDuration = validityDuration
        };

        try
        {
            var response = await _client.PostAsJsonAsync($"{_tokengenUrl}/forward", request);
            response.EnsureSuccessStatusCode();

            var result = await response.Content.ReadFromJsonAsync<TokenResponse>();
            if (result?.Token == null)
            {
                throw new Exception("Token generation failed: Empty response");
            }

            return result.Token;
        }
        catch (HttpRequestException ex)
        {
            throw new Exception($"Failed to connect to tokengen server at {_tokengenUrl}: {ex.Message}", ex);
        }
    }

    /// <summary>
    /// Generates a KDC proxy token for Kerberos authentication through the gateway.
    /// </summary>
    /// <param name="krbRealm">Kerberos realm (e.g., "AD.EXAMPLE.COM")</param>
    /// <param name="krbKdc">KDC address with protocol (e.g., "tcp://dc.ad.example.com:88")</param>
    /// <param name="validityDuration">Token validity in seconds (default: 3600)</param>
    /// <returns>A JWT token string</returns>
    public async Task<string> GenerateKdcToken(
        string krbRealm,
        string krbKdc,
        int validityDuration = 3600)
    {
        var request = new KdcTokenRequest
        {
            KrbRealm = krbRealm,
            KrbKdc = krbKdc,
            ValidityDuration = validityDuration
        };

        try
        {
            var response = await _client.PostAsJsonAsync($"{_tokengenUrl}/kdc", request);
            response.EnsureSuccessStatusCode();

            var result = await response.Content.ReadFromJsonAsync<TokenResponse>();
            if (result?.Token == null)
            {
                throw new Exception("KDC token generation failed: Empty response");
            }

            return result.Token;
        }
        catch (HttpRequestException ex)
        {
            throw new Exception($"Failed to generate KDC token from {_tokengenUrl}: {ex.Message}", ex);
        }
    }

    /// <summary>
    /// Checks if the tokengen server is reachable.
    /// </summary>
    /// <returns>True if server is reachable, false otherwise</returns>
    public async Task<bool> IsServerReachable()
    {
        try
        {
            var response = await _client.GetAsync(_tokengenUrl);
            return response.IsSuccessStatusCode || response.StatusCode == System.Net.HttpStatusCode.NotFound;
        }
        catch
        {
            return false;
        }
    }

    public void Dispose()
    {
        _client?.Dispose();
    }

    // Request/Response DTOs
    private class RdpTlsTokenRequest
    {
        [JsonPropertyName("dst_hst")]
        public string DstHst { get; set; } = string.Empty;

        [JsonPropertyName("prx_usr")]
        public string PrxUsr { get; set; } = string.Empty;

        [JsonPropertyName("prx_pwd")]
        public string PrxPwd { get; set; } = string.Empty;

        [JsonPropertyName("dst_usr")]
        public string DstUsr { get; set; } = string.Empty;

        [JsonPropertyName("dst_pwd")]
        public string DstPwd { get; set; } = string.Empty;

        [JsonPropertyName("jet_aid")]
        public string? JetAid { get; set; }

        [JsonPropertyName("validity_duration")]
        public int ValidityDuration { get; set; }
    }

    private class ForwardTokenRequest
    {
        [JsonPropertyName("dst_hst")]
        public string DstHst { get; set; } = string.Empty;

        [JsonPropertyName("jet_ap")]
        public string JetAp { get; set; } = "rdp";

        [JsonPropertyName("jet_rec")]
        public bool JetRec { get; set; }

        [JsonPropertyName("validity_duration")]
        public int ValidityDuration { get; set; }
    }

    private class KdcTokenRequest
    {
        [JsonPropertyName("krb_realm")]
        public string KrbRealm { get; set; } = string.Empty;

        [JsonPropertyName("krb_kdc")]
        public string KrbKdc { get; set; } = string.Empty;

        [JsonPropertyName("validity_duration")]
        public int ValidityDuration { get; set; }
    }

    private class TokenResponse
    {
        [JsonPropertyName("token")]
        public string? Token { get; set; }
    }
}
