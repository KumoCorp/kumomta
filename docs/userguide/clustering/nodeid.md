---
description: Configure the KumoMTA NodeId, a per-instance UUID that identifies nodes in a cluster, aids reporting, and persists or falls back to a generated ID.
---

# Node ID

Each KumoMTA (`kumod`) instance can have its own `NodeId`, which is a UUID
intended to identify that specific instance within your own local cluster, help
with disambiguation during reporting, and also for future configuration
management/provisioning related functionality.

The `NodeId` is reported as the `nodeid` field in the [Log
Record](../../reference/log_record.md).

In the default configuration, KumoMTA will use the file
`/opt/kumomta/etc/.nodeid` to persist the `NodeId`.  If that file doesn't
exist, a new ID will be generated and stored in that location.

If persisting the `NodeId` isn't possible, we fall back to generating an
id as described in the section below.

## Environment Variables

The following environment variables influence the Node ID:

  * `KUMO_NODE_ID` - if this is set to a valid UUID, its value will be used as
    the `NodeId` for the instance.  You might contrive for your orchestration
    system to set this if you want total control over the relationship between
    the machine and node id.

  * `KUMO_NODE_ID_PATH` - this can be set to an alternative location into which
    the nodeid should be stored.  If this is not set, the path is assumed to be
    `/opt/kumomta/etc/.nodeid`.  If the path is not writable for some reason
    (e.g., permission denied), then a fallback nodeid will be computed.

## Fallback Node ID

If `NodeId` cannot be persisted then the following fallback procedure will be
used to compute an ID that will have a consistent value across restarts of
the kumod process:

  * First attempt to determine the MAC address of a physical network interface
    on the system, as described in [MAC Address Selection](#mac-address-selection)
    below.

  * If we cannot determine the MAC address then use the POSIX `gethostid(3)`
    call to obtain a 32-bit stable identifier for the system, which is extended
    to 6 bytes to make it the same size and shape as a MAC address.

The MAC address bytes are then used to compute a V1 (time based) UUID with a
fixed timestamp, which produces a UUID that looks something like
`00000000-0000-1000-8000-XXXXXXXXXXXX` where the X's are the hex digits from
the MAC address.

Neither the true MAC address nor especially the `gethostid(3)` fallback are
ideal if the network interface might change across the lifetime of a logical
instance, so we recommend fixing any permission errors that might be preventing
persisting a true random UUID or alternatively, adjusting your node
provisioning to pre-define a `KUMO_NODE_ID` environment variable if you have
stronger opinions about how you want to provision and manage these things.

## MAC Address Selection

{{since('dev')}}

The MAC address is used by the fallback Node ID above, and also identifies the
node within the ids assigned to spooled messages. It must be distinct between
hosts to avoid multiple hosts attempting to assign the same spool id.  While
KumoMTA has logic to detect and avoid this sort of collision, it results in
lower performance.

By default KumoMTA selects the first physical network interface, skipping
loopback and virtual devices such as container bridges (`docker0`), veth pairs,
and tunnels, whose addresses are frequently identical across otherwise separate
machines. On some hosts (for example certain cloud or containerized setups) this
automatic choice can still select an interface whose MAC is not unique. The
following environment variables let you override the selection:

  * `KUMO_MAC_INTERFACE` - set this to the name of a network interface (for
    example `eth0` or `ens5`) and its MAC address will be used.

  * `KUMO_MAC_ADDRESS` - set this to a literal MAC address, in the usual colon
    or hyphen separated hex form, to use verbatim. Each host must be given a
    distinct value in order to prevent collisions.

The resolved MAC address, along with how it was selected, is written to the log
when the node starts up.

In earlier versions of KumoMTA the MAC address was always taken from the first
interface reported by the operating system, with no skipping of virtual devices
and no way to override the choice; the `KUMO_MAC_INTERFACE` and
`KUMO_MAC_ADDRESS` environment variables did not exist.

