/**
 * Helpers for Microsoft Entra ID sign-in (RDS AAD Auth).
 *
 * The application acquires the RDP access token and the AAD nonce from the Microsoft identity
 * platform and signs the RDP assertion with a proof-of-possession key that stays in the browser.
 * See [MS-RDPBCGR] sections 2.2.18.2.1 (RDP Assertion) and 4.11.4 (Acquiring an RDP Access Token).
 */

const KEY_ALGORITHM = 'RSASSA-PKCS1-v1_5';

const encoder = new TextEncoder();

function base64Url(bytes: Uint8Array): string {
    let binary = '';
    for (const byte of bytes) {
        binary += String.fromCharCode(byte);
    }
    return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function rsaPublicJwk(publicKey: CryptoKey): Promise<{ kty: 'RSA'; e: string; n: string }> {
    const jwk = await crypto.subtle.exportKey('jwk', publicKey);
    if (jwk.kty !== 'RSA' || jwk.e === undefined || jwk.n === undefined) {
        throw new Error('the Entra proof-of-possession key must be an RSA key');
    }
    return { kty: 'RSA', e: jwk.e, n: jwk.n };
}

/**
 * Generates the 2048-bit RSA proof-of-possession key the RDP access token is bound to.
 * The private key cannot be exported.
 */
export function generateEntraPopKey(): Promise<CryptoKeyPair> {
    return crypto.subtle.generateKey(
        {
            name: KEY_ALGORITHM,
            modulusLength: 2048,
            publicExponent: new Uint8Array([1, 0, 1]),
            hash: 'SHA-256',
        },
        false,
        ['sign', 'verify'],
    );
}

/**
 * Computes the `req_cnf` parameter of the token request for a proof-of-possession key:
 * the Base64URL encoding of `{"kid":"<RFC 7638 thumbprint>"}`.
 */
export async function entraReqCnf(publicKey: CryptoKey): Promise<string> {
    const { e, n } = await rsaPublicJwk(publicKey);
    // RFC 7638 hashes the required members in lexicographic order without whitespace.
    const canonical = `{"e":"${e}","kty":"RSA","n":"${n}"}`;
    const thumbprint = new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(canonical)));
    return base64Url(encoder.encode(JSON.stringify({ kid: base64Url(thumbprint) })));
}

export type EntraAssertionParams = {
    /** RDP access token bound to `key` through `req_cnf`. */
    accessToken: string;
    /**
     * Resource URI the access token was acquired for, without the `/user_impersonation` suffix:
     * `ms-device-service://termsrv.wvd.microsoft.com/id/<device ID>` or `.../name/<hostname>`.
     */
    resourceUri: string;
    /** Nonce the server sent after the TLS handshake. */
    serverNonce: string;
    /** Nonce obtained from the token endpoint with `grant_type=srv_challenge`. */
    aadNonce: string;
    /** Proof-of-possession key the access token is bound to. */
    key: CryptoKeyPair;
    /** Current time in seconds since the Unix epoch; defaults to the system clock. */
    unixTimeSecs?: number;
};

/**
 * Signs the RDP assertion for one connection: an RS256 compact JWS over the access token and
 * both nonces. Pass the result to the callback of the `entraAssertion` extension.
 */
export async function signEntraAssertion(params: EntraAssertionParams): Promise<string> {
    const header = { alg: 'RS256', kid: await entraReqCnf(params.key.publicKey) };
    // The members follow the order of the specification.
    const payload = {
        ts: String(params.unixTimeSecs ?? Math.floor(Date.now() / 1000)),
        at: params.accessToken,
        u: params.resourceUri,
        nonce: params.serverNonce,
        cnf: { jwk: await rsaPublicJwk(params.key.publicKey) },
        client_claims: JSON.stringify({ aad_nonce: params.aadNonce }),
    };

    const signingInput = `${base64Url(encoder.encode(JSON.stringify(header)))}.${base64Url(
        encoder.encode(JSON.stringify(payload)),
    )}`;
    const signature = await crypto.subtle.sign(KEY_ALGORITHM, params.key.privateKey, encoder.encode(signingInput));

    return `${signingInput}.${base64Url(new Uint8Array(signature))}`;
}
