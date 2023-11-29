# Connection Metadata Object

{{since('2023.08.22-4d895015')}}

This object represents a collection of metadata keys and values that
are associated with an established incoming SMTP connection.

KumoMTA populates a small number of predefined fields (see below), and allows
your policy scripts the ability to read those values as well as write (and
read!) additional values as needed by the local policy.  For instance, you may
decide to compute a value after EHLO has been processed by
[smtp_server_ehlo](events/smtp_server_ehlo.md) and access that value in a
later SMTP event handler.

Prior to calling [smtp_server_message_received](events/smtp_server_message_received.md),
KumoMTA will copy the values from the connection metadata and use those to populate
the [message](message/index.md) metadata.

The `get_meta` and `set_meta` methods shown below are used to read and write
metadata values.

## Predefined Connection Metadata Values

The following values are predefined by KumoMTA:

|Name|Purpose|Since|
|----|-------|-----|
|`reception_protocol`|indicates the reception protocol, such as `ESMTP`|{{since('2023.08.22-4d895015', inline=True)}}|
|`received_via`|indicates the IP:port of the KumoMTA listener that is handling this session|{{since('2023.08.22-4d895015', inline=True)}}|
|`received_from`|indicates the IP:port of the sending or peer machine in this session|{{since('2023.08.22-4d895015', inline=True)}}|
|`hostname`|A copy of the effective value of the hostname set by [kumo.start_esmtp_listener](kumo/start_esmtp_listener.md#hostname)|{{since('2023.11.28-b5252a41', inline=True)}}|

## Available Methods

### `conn_meta:get_meta(name)`

Returns the value associated with *name*, or `nil` if no such value has been defined.
Values may be predefined by KumoMTA, or may be set by policy scripts using `conn_meta:set_meta()`.


### `conn_meta:set_meta(name, value)`

Sets the value associated with *name* to *value*.  Value must be serializable as JSON; it can be simple
strings or numbers, but may also be an array or object value.
