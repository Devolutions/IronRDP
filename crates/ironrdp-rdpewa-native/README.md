# ironrdp-rdpewa-native

Windows WebAuthn API backend for the IronRDP [MS-RDPEWA] client.

Implements `RdpewaClientHandler` using `webauthn.dll` exports
(`WebAuthNAuthenticatorMakeCredential`, `WebAuthNAuthenticatorGetAssertion`, …).

[MS-RDPEWA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpewa/
