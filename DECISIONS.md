## 2026-08-07 — Reuse RDCleanPath v1 for VMConnect

Refactor RDCleanPath VMConnect support to reuse the existing v1 `preconnection_blob` field. Do not
add `server_preconnection_pdu` or VERSION_2. Treat X.224 request/response as optional in the typed
model: present means the ordinary RDP front; absent means PCB → TLS, followed by client-driven
CredSSP → X.224.
