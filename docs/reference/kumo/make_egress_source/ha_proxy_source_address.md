# ha_proxy_source_address

Optional string.

Specifies the source address that the HA Proxy server should use when
initiating a connection.

!!! note
   The HA Proxy protocol doesn't provide a mechanism for reporting
   whether binding to this address was successful.  From the perspective
   of KumoMTA, invalid proxy configuration will appear as a timeout
   with no additional context.  We recommend using SOCKS5 instead
   of HA proxy, as the SOCKS5 protocol is better suited for outbound
   connections.


