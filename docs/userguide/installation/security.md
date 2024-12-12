# Security Considerations

This page summarizies the key considerations for deploying a secure installation of KumoMTA.

## Operating System

The `.deb` and `.rpm` packages that we provide are preconfigured to create a
service account named `kumod` and grant that user write access to the
suggested default spool and log locations.

The service is launched as the root user in order to bind to the privileged
SMTP (port 25). The service immediately on startup, before taking any other
action, drops all of the root privileges except for `CAP_NET_BIND_SERVICE`
(which is required to bind to port 25) and then switches its user id to the
`kumod` user.

## Spool, Log and DKIM Directory Permissions

The suggested default locations for these are:

* `/var/spool/kumomta`
* `/var/log/kumomta`
* `/opt/kumomta/etc/dkim`

In the standard packaging these locations are deployed with owner `kumod`,
group `kumod` and mode `2770` to constrain access to just the `kumod` user.

If you mount or otherwise select separate locations for these functions, it is
recommended that you apply the same ownership and mode in order that other
programs on the system are not able to exfiltrate information about the
traffic, message content, or DKIM signing credentials.

## Policy Directory Permissions

The suggested default locations for the policy files are:

* `/opt/kumomta/etc/policy`

In the standard packaging these locations are deployed with owner `kumod`,
group `kumod` and mode `755`.

It is recommended that you avoid encoding secrets directly into files
contained with the `/opt/kumomta/etc/policy` location and instead deploy
[using a secret manager such as HashiCorp vault](../policy/hashicorp_vault.md), or alternatively, deploy those secrets in a similar way to the DKIM keys mentioned above.

## Administrative Access

Administration is carried out via an HTTP listener that you must explicitly
configure. The suggested default configuration for the listener is to bind only
on the IPv4 loopback interface on port 8000, and for the loopback address
to be considered to be a trusted host.

That means that any process or user on the local host can issue administrative
commands to the kumod instance in the default configuration.

You can widen or restrict this access by changing the [listen
addresses](../../reference/kumo/start_http_listener/listen.md) on which you run
http listeners and changing the
[trusted_hosts](../../reference/kumo/start_http_listener/trusted_hosts.md)

## SMTP Relaying

It is important to avoid allowing arbitrary sources of traffic to inject mail
and relay it anywhere. The default configuration is to prevent relaying except
for the [relay_hosts](../../reference/kumo/start_esmtp_listener/relay_hosts.md)
defined in the SMTP listener.

You can further control relaying through a combination of [SMTP
Authentication](../policy/inbound_auth.md) and [relay
domains](../configuration/domains.md).

## Authenticating Incoming Requests

See the following sections of the docs:

* [Inbound SMTP Authentication](../policy/inbound_auth.md)
* [Inbound HTTP Basic Authentication](../../reference/events/http_server_validate_auth_basic.md)

## Inbound TLS

Both the SMTP and HTTP listener will automatically generate a self-signed
certificate if you haven't explicitly provisioned a trusted certificate.
This allows communication with the service to proceed on an encrypted
connection, but without trust.  The self-signed certificate generated
for this purpose is held in-memory and will be regenerated when the
service is restarted.

It is strongly recommended that you provision your own trusted certificates
for your listeners.

## Outbound TLS

The default configuration in the shaping helper for outgoing SMTP is to enable
`Opportunistic` TLS, which is to make use of TLS if the destination host
advertises it, but only if the certificate is trusted.

Unfortunately, there are a large number of destination sites with poorly
maintained TLS, so many kumomta users choose to deploy with
`OpportunisticInsecure` TLS as a default, which will try to use TLS if
available, but will allow communicating in clear text if there are any issues
trying to establish the connection.  That rationale for this choice is that
having an encrypted transport is more private than not having it, even if that
means that you cannot assert that the connection is to the intended
destination, especially if you are prepared to send the message in clear text
as a fallback anyway.

Care needs to be taken when employing `OpportunisticInsecure` as it
introduces the risk of a Man-in-the-Middle attack that can intercept
the outgoing traffic.

There are three main ways in which you can manage that risk:

* Use [MTA-STS](../../reference/kumo/make_egress_path/enable_mta_sts.md) (which
  is enabled by default) to allow a destination site to publish its own choice
  on TLS policy via a well-known HTTPS endpoint.  Sites that deploy MTA-STS can
  ensure that the TLS is set to required even if your default policy is
  opportunistic.
* For well-known sites with working TLS, such as Google, override the
  opportunistic TLS with required TLS in your [shaping configuration](../configuration/trafficshaping.md).
* Consider enabling [DANE](../../reference/kumo/make_egress_path/enable_dane.md), which is
  similar in effect to MTA-STS, using signed DNS records instead of publishing
  its policy via HTTPS. It requires working and trusted DNSSEC
  capability in your infrastructure. Since we can't guarantee that it will work
  out of the box without the operator explicitly confirming that functionality,
  this is not enabled by default.
