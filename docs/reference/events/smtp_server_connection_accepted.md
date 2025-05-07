# `kumo.on('smtp_server_connection_accepted', function(conn_meta))`

{{since('2025.05.06-b29689af')}}

Called by the ESMTP server when a new server session has accepted
a connection from a client.

This event is triggered *before* sending the initial banner response, giving
you the opportunity to decide whether to reject the connection, or continue.

If you do not reject the connection, then the server will continue with
returning the banner to the client as normal.

The [Connection Metadata](../connectionmeta.md) object is passed as
the only parameter, which can be used to determine information about
the peer, and can be modified to track additional context throughout
the lifetime of this particular connection.

```lua
kumo.on('smtp_server_connection_accepted', function(conn_meta)
  local peer = conn_meta:get_meta 'received_from'
  -- is_peer_deny_listed is some hypothetical function you
  -- define that will check to see if you want to allow this
  -- connection to continue
  if is_peer_deny_listed(peer) then
    kumo.reject(421, string.format('service not accepted from %s', peer))
  end
end)
```

