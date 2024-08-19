# Predefined Metadata

KumoMTA provides the ability to set and retrieve metadata at both the connection and message level.

By leveraging metadata, information can be made available to policy running at different phases in the life of a message, where the [connection metadata](./connectionmeta.md) is used for data that is shared in common with all messages injected over a given connection, and the [message metadata](./message/set_meta.md) is for all data related to a given individual message.

There are get and set functions available for both connection and message metadata, and when a message is received **all connection metadata is also copied into the message metadata**, meaning that for retrieving connection metadata the user can opt to only access message metadata for any value that doesn't change over the life of the connection.

The following metadata values are predefined by KumoMTA and are available to retrieve:

<style>
table tbody tr td:nth-of-type(2) {
  white-space: nowrap;
}
</style>

|Scope|Name|Purpose|Since|
|----|----|-------|-----|
|Connection|`reception_protocol`|indicates the reception protocol, such as `ESMTP`|{{since('2023.08.22-4d895015', inline=True)}}|
|Connection|`received_via`|indicates the IP:port of the KumoMTA listener that is handling this session|{{since('2023.08.22-4d895015', inline=True)}}|
|Connection|`received_from`|indicates the IP:port of the sending or peer machine in this session|{{since('2023.08.22-4d895015', inline=True)}}|
|Connection|`hostname`|A copy of the effective value of the hostname set by [kumo.start_esmtp_listener](kumo/start_esmtp_listener/hostname.md)|{{since('2023.11.28-b5252a41', inline=True)}}|
|Connection|`authn_id`|the authentication id if the message was received via authenticated SMTP||
|Connection|`authz_id`|the authorization id if the message was received via authenticated SMTP||
|Message|`queue`|specify the name of the queue to which the message will be queued. Must be a string value.||
|Message|`tenant`|specify the name/identifier of the tenant, if any. Must be a string value.||
|Message|`campaign`|specify the name/identifier of the campaign. Must be a string value.||
|Message|`routing_domain`|Overrides the domain of the recipient domain for routing purposes.|{{since('2023.08.22-4d895015', inline=True)}}|
