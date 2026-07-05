# Post-auth connection binding for multi-user servers

`wrdp` follows the same multi-user architecture model as `xrdp-sesman`: a
single public RDP listener authenticates the client first, then delegates the
connection to a per-user desktop/session stack.

That model needs a server hook that runs after credentials have been accepted
but before display updates and input dispatch begin. The hook lets a server
start or locate the authenticated user's session and then replace placeholder
handlers with display/input handlers bound to that session.

The `ConnectionBinder` API keeps protocol ownership inside IronRDP while
allowing downstream servers to keep user/session lifecycle code outside the RDP
state machine.
