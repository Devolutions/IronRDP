# Reactivation credential cache scope

During Deactivation-Reactivation some clients do not send a second credentials
PDU. A server that binds display/input handlers after authentication still needs
the identity accepted earlier on the same TCP connection.

The cache introduced here is deliberately scoped to `accept_finalize()`, i.e. to
one TCP connection. It allows same-connection reactivation to reuse the validated
identity but prevents a new TCP connection from inheriting credentials accepted
on a previous connection.
