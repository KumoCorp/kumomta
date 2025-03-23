# max_connections

{{since('2024.09.02-c5476b89')}}

Specifies the maximum number of concurrent connections that are permitted
to this listener. Connections above this will be accepted and then closed
immediately with a `421 4.3.2` response.

The default value for this is `32768`, which is approximately half of
the possible number of ports for a given IP address, leaving the other
half available for outgoing connections.

Each time a connection is denied due to hitting this limit, the
`total_connections_denied` counter is incremented for the `esmtp_listener`
service.

In earlier releases, there was no kumod-controlled upper bound on the
number of connections, and as many as the kernel allowed would be
permitted.

!!! note
    This option cannot be used in dynamic listener contexts such as within
    [via](via.md), [peer](peer.md) or within the parameters returned from
    [smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md).
    It can only be used directly at the top level within the
    `kumo.start_esmtp_listener` call.

