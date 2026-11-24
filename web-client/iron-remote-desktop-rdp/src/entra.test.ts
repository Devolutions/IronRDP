import { describe, expect, it } from 'vitest';
import { entraReqCnf, generateEntraPopKey, signEntraAssertion } from './entra';

const KEY_ALGORITHM = { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' };

function decodeBase64Url(value: string): Uint8Array {
    const padded = value
        .replace(/-/g, '+')
        .replace(/_/g, '/')
        .padEnd(Math.ceil(value.length / 4) * 4, '=');
    return Uint8Array.from(atob(padded), (char) => char.charCodeAt(0));
}

function decodeJson(value: string): unknown {
    return JSON.parse(new TextDecoder().decode(decodeBase64Url(value)));
}

describe('entraReqCnf', () => {
    // oracle: RFC 7638 section 3.1, the example JWK and its thumbprint
    it('wraps the RFC 7638 thumbprint of the public key', async () => {
        const publicKey = await crypto.subtle.importKey(
            'jwk',
            {
                kty: 'RSA',
                e: 'AQAB',
                n:
                    '0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3' +
                    'oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zg' +
                    'dAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csF' +
                    'Cur-kEgU8awapJzKnqDKgw',
            },
            KEY_ALGORITHM,
            true,
            ['verify'],
        );

        const reqCnf = await entraReqCnf(publicKey);

        expect(decodeJson(reqCnf)).toEqual({ kid: 'NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs' });
    });
});

describe('signEntraAssertion', () => {
    const params = {
        accessToken: 'access-token-under-test',
        resourceUri: 'ms-device-service://termsrv.wvd.microsoft.com/id/5d9d9c9f-1805-42e3-8c87-15208f7e175d',
        serverNonce: 'server-nonce-under-test',
        aadNonce: 'aad-nonce-under-test',
        unixTimeSecs: 1_700_000_000,
    };

    // oracle: spec MS-RDPBCGR section 2.2.18.2.1 (RDP Assertion), members and their order
    it('carries the token, the resource and both nonces in specification order', async () => {
        const key = await generateEntraPopKey();

        const assertion = await signEntraAssertion({ ...params, key });
        const [header, payload] = assertion.split('.');

        expect(decodeJson(header)).toEqual({ alg: 'RS256', kid: await entraReqCnf(key.publicKey) });

        const jwk = await crypto.subtle.exportKey('jwk', key.publicKey);
        const claims = decodeJson(payload) as Record<string, unknown>;
        expect(Object.keys(claims)).toEqual(['ts', 'at', 'u', 'nonce', 'cnf', 'client_claims']);
        expect(claims).toEqual({
            ts: '1700000000',
            at: params.accessToken,
            u: params.resourceUri,
            nonce: params.serverNonce,
            cnf: { jwk: { kty: 'RSA', e: jwk.e, n: jwk.n, alg: 'RS256' } },
            client_claims: '{"aad_nonce":"aad-nonce-under-test"}',
        });
    });

    // oracle: RFC 7515 section 3.1, the signature covers "<header>.<payload>"
    it('is signed by the proof-of-possession key', async () => {
        const key = await generateEntraPopKey();

        const assertion = await signEntraAssertion({ ...params, key });
        const [header, payload, signature] = assertion.split('.');

        const valid = await crypto.subtle.verify(
            KEY_ALGORITHM,
            key.publicKey,
            decodeBase64Url(signature),
            new TextEncoder().encode(`${header}.${payload}`),
        );
        expect(valid).toBe(true);
    });
});
