# `kumo.make_egress_path { PARAMS }`

Constructs a configuration object that specifies how traffic travelling the
path from a *source* to a *site* will behave.

This function should be called from the
[get_egress_path_config](../events/get_egress_path_config.md) event handler to provide the
configuration for the requested site.

The following keys are possible:

## connection_limit

Specifies the maximum number of concurrent connections that will be made from
the current MTA machine to the destination site.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = 32,
  }
end)
```

## consecutive_connection_failures_before_delay

Each time KumoMTA exhausts the full list of hosts for the destination it
increments a `consecutive_connection_failures` counter. When that counter
exceeds the `consecutive_connection_failures_before_delay` configuration value,
KumoMTA will then delay all of the messages currently in the ready queue,
generating a transient failure log record with code `451 4.4.1 No answer from
any hosts listed in MX`.

The default value for this setting is 100.

## ehlo_domain

Optional string. Specifies the EHLO domain when initiating a connection to the
destination. The default value is the `ehlo_domain` specified by
[make_egress_source](make_egress_source.md), if any, otherwise, the local
machine hostname.

## enable_tls

Controls whether and how TLS will be used when connecting to the destination.
Possible values are:

* `"Opportunistic"` - use TLS if advertised by the `EHLO` response. If the peer
  has invalid or self-signed certificates, then the delivery will fail. KumoMTA
  will NOT fallback to not using TLS on that same host.

* `"OpportunisticInsecure"` - use TLS if advertised by the `EHLO` response.
  Validation of the certificate will be skipped. Not recommended for sending to
  the public internet; this is intended for local or lab testing scenarios.

* `"Required"` - Require that TLS be advertised in the `EHLO` response. The
  remote host must have valid certificates in order to deliver to the site.

* `"RequiredInsecure"` - Require that TLS be advertised in the `EHLO` response.
  Validation of the certificate will be skipped.  Not recommended for sending
  to the public internet; this is intended for local or lab testing scenarios.

* `"Disabled"` - do not try to use TLS.

The default value is `"Opportunistic"`.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
  }
end)
```

## enable_mta_sts

{{since('2023.11.28-b5252a41', indent=True)}}
    When set to `true` (which is the default), the
    [MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461) policy for the
    destination domain will be used to adjust the effective value of `enable_tls`.

    If the policy is set to `"enforce"`, then, assuming that the candidate
    MX host name matches the policy, the connection will be made with
    `enable_tls="Required"`.  If the host name does not match, the candidate
    MX host will be not be used.

    If the policy is set to `"testing"`, then the connection will be made
    with `enable_tls="OpportunisticInsecure"`.

    If the policy is set to `"none"`, then your configured value for `enable_tls`
    will be used.

    If `enable_dane=true` and `TLSA` records are present, then any MTA-STS policy
    will be ignored.

## enable_dane

{{since('2023.11.28-b5252a41', indent=True)}}
    When set to `true` (the default is `false`), then `TLSA` records will be
    resolved securely to determine the destination site policy for TLS according
    to [DANE](https://datatracker.ietf.org/doc/html/rfc7672).

    If TLSA records are available, then the effective value of `enable_tls` will
    be treated as though it were set to `"Required"` and the OpenSSL DANE implementation
    will be used to verify the server certificate against the TLSA records found
    in DNS.

    Use of DANE also *requires* functioning DNSSEC in your DNS resolver; you
    will need to configure the `libunbound` resolver to successfully use DANE:

    ```lua
    kumo.on('init', function()
        kumo.dns.configure_unbound_resolver {
            options = {
                -- Enable DNSSEC
                validate = true,
            },
            -- By default, if you omit `name_servers`, unbound will
            -- resolve via the root resolvers.
            -- We strongly recommend deploying local caching nameservers
            -- and referencing them here:
            -- name_servers = { '1.1.1.1:53' },
        }
    end)
    ```
The following nine settings control the timeouts waiting for responses to various SMTP commands.

The value is specified as a integer in seconds, or as a string using syntax
like `"2min"` for a two minute duration.

## banner_timeout

{{since('dev')}}

How long to wait between a connection being established and receiving a 220
from a receiving host. The default is `60s`.

In earlier versions of KumoMTA, this was rolled together into the `connect_timeout`.

## connect_timeout

How long to wait between starting an SMTP connection and receiving a 220 from a
receiving host. The default is `60s`.

{{since('dev', inline=True)}}
    The `connect_timeout` is now purely focused on the time it takes to
    establish a working connection. The time allowed for receiving the
    initial 220 banner has been separated out into `banner_timeout`.

## starttls_timeout
How long to wait for a response after issuing a STARTTLS comand. The default is `5s`.

## ehlo_timeout
How long to wait for a response after issuing an EHLO command.  The default is `300s`.

## mail_from_timeout
How long to wait for a response after issuing a MAIL FROM command. The default is `300s`.

## rcpt_to_timeout
How long to wait for a response after issuing a RCPT TO command. The default is `300s`.

## data_timeout
How long to wait for a response after issuing a DATA command. The default is `30s`.

## data_dot_timeout
How long to wait for a response after issuing a closing "." at the end of the DATA field. The default is `60s`.

## rset_timeout
How long to wait for a response after issuing a RSET command. The default is `5s`.

## idle_timeout
How long a connection will remain open and idle, waiting to be
reused for another delivery attempt, before being closed.

The value is specified as a integer in seconds, or as a string using syntax
like `"2min"` for a two minute duration. The default is `60s`.


```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    idle_timeout = 60,
  }
end)
```

## max_connection_rate

Optional string.

Specifies the maximum permitted rate at which connections can be established
from this source to the corresponding destination site.

The value is of the form `quantity/period`
where quantity is a number and period can be a measure of time.

Examples of throttles:

```
"10/s" -- 10 per second
"10/sec" -- 10 per second
"10/second" -- 10 per second

"50/m" -- 50 per minute
"50/min" -- 50 per minute
"50/minute" -- 50 per minute

"1,000/hr" -- 1000 per hour
"1_000/h" -- 1000 per hour
"1000/hour" -- 1000 per hour

"10_000/d" -- 10,000 per day
"10,000/day" -- 10,000 per day
```

Throttles are implemented using a Generic Cell Rate Algorithm.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    max_connection_rate = '100/min',
  }
end)
```

If the throttle is exceeded and the delay before a connection be established
is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

## max_deliveries_per_connection

Optional number.

If set, no more than this number of messages will be attempted on any
given connection.

|Version|Default|
|-------|-------|
|{{since('2023.08.22-4d895015', inline=True)}}|The default is 1024|
|Prior versions|The default is unlimited|

## max_message_rate

Optional string.

Specifies the maximum permitted rate at which messages can be delivered
from this source to the corresponding destination site.

The throttle is specified the same was as for `max_connection_rate` above.

If the throttle is exceeded and the delay before the current message can be
sent is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

This option is distinct from [the scheduled queue
max_message_rate](make_queue_config.md#max_message_rate) option in that the
scheduled queue option applies to a specific scheduled queue, whilst this
egress path option applies to the ready queue for a specific egress path,
through which multiple scheduled queues send out to the internet.

If you have configured `max_message_rate` both here and in a scheduled queue,
the effective maximum message rate will be the lesser of the two values; both
constraints are applied independently from each other at different stages
of processing.

## max_ready

Specifies the maximum number of messages that can be in the *ready queue*.
The ready queue is the set of messages that are immediately eligible for delivery.

If a message is promoted from its delayed queue to the ready queue and it would
take the size of the ready queue above *max_ready*, the message will be delayed
by a randomized interval of up to 60 seconds and placed back into the scheduled
queue before being considered again.

Moving a message from *ready* to *scheduled* as a result of hitting this limit
may trigger disk IO to save the content of the message if the message was
received with deferred spooling enabled.  In addition, other in-memory state
is discarded to reduce memory utilization, and it will need to be re-loaded
from the spool when the message is tried again later.

The default for `max_ready` is 1024 messages.

Raising the limit will increase RAM utilization in exchange for decreasing
the IO load to your spool storage.

## prohibited_hosts

A CIDR list of hosts that should be considered "poisonous", for example, because
they might cause a mail loop.

When resolving the hosts for the destination MX, if any of the hosts are
present in the `prohibited_hosts` list then the ready queue will be immediately
failed with a `550 5.4.4` status.

## skip_hosts

A CIDR list of hosts that should be removed from the list of hosts returned
when resolving the MX for the destination domain.

This can be used for example to skip a host that is experiencing issues.

If all of the hosts returned for an MX are filtered out by `skip_hosts` then
the ready queue will be immediately failed with a `550 5.4.4` status.

## smtp_port

Specifies the port to connect to when making an SMTP connection to a destination
MX host.

The default is port 25.

See also [kumo.make_egress_source().remote_port](make_egress_source.md#remote_port)

## smtp_auth_plain_username

When set, connecting to the destination requires a successful AUTH PLAIN using the
specified username.

AUTH PLAIN will only be attempted if TLS is also enabled, unless
`allow_smtp_auth_plain_without_tls = true`. This is to prevent leaking
of the credential over an unencrypted link.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'scott',
    -- The password can be any keysource value
    smtp_auth_plain_password = {
      key_data = 'tiger',
    },
  }
end)
```

## smtp_auth_plain_password

Specifies the password that should be used together with `smtp_auth_plain_username`
when an authenticated SMTP connection is desired.

The value is any [keysource](../keysource.md), which allows for specifying the
password inline in the configuration file, or managing it via a credential manager
such as HashiCorp Vault.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'scott',
    -- The password can be any keysource value.
    -- Here we are loading the credential for the domain
    -- from HashiCorp vault
    smtp_auth_plain_password = {
      vault_mount = 'secret',
      vault_path = 'smtp-auth/' .. domain,
    },
  }
end)
```

## allow_smtp_auth_plain_without_tls

Optional boolean. Defaults to `false`.

When `false`, and the connection is not using TLS, SMTP AUTH PLAIN will be
premptively failed in order to prevent the credential from being passed over
the network in clear text.

You can set this to `true` to allow sending the credential in clear text.

!!! danger
    Do not enable this option on an untrusted network, as the credential
    will then be passed in clear text and visible to anyone else on the
    network

## suspended

{{since('2023.08.22-4d895015')}}

Optional boolean. Defaults to `false`.

When set to `true`, this pathway will not be used to send mail.

This option is present primarily to facilitate traffic shaping automation.

!!! warning
    This option is deprecated and will be removed in a future release.
    It has been subsumed by realtime TSA suspension updates.
    It no longer has any effect.

## tls_prefer_openssl

{{since('dev')}}

Optional boolean. Defaults to `false`.

When set to `true`, this pathway will prefer to use OpenSSL as the TLS
implementation.

When set to `false`, this pathway will prefer to use rustls as the TLS
implementation, unless DANE is enabled and TLSA records are present, in which
case OpenSSL will be used.

## openssl_cipher_list

{{since('dev')}}

Optional string.

If set, then the value will be used to configure the set of ciphers used by
OpenSSL for TLS protocol version lower than 1.3.

OpenSSL is used as described under the
[tls_prefer_openssl](#tls_prefer_openssl) option above.

The format of the string is [discussed in the OpenSSL ciphers
documentation](https://www.openssl.org/docs/man1.1.1/man1/ciphers.html)

## openssl_cipher_suites

{{since('dev')}}

Optional string.

If set, then the value will be used to configure the set of ciphers used by
OpenSSL for TLS protocol version 1.3.

OpenSSL is used as described under the
[tls_prefer_openssl](#tls_prefer_openssl) option above.

The format consists of TLSv1.3 cipher suite names separated by `:` characters
in order of preference.

## openssl_options

{{since('dev')}}

Optional string.

If set, then the value will be used to configure openssl option flags.

OpenSSL is used as described under the
[tls_prefer_openssl](#tls_prefer_openssl) option above.

The format of the string is the set of possible option names separated by `|` characters.

Option names are:

* `ALL` - A “reasonable default” set of options which enables compatibility flags.
* `NO_QUERY_MTU` - Do not query the MTU.  Only affects DTLS connections.
* `COOKIE_EXCHANGE` - Enables Cookie Exchange as described in [RFC 4347 Section
  4.2.1](https://tools.ietf.org/html/rfc4347#section-4.2.1).  Only affects DTLS
  connections.
* `NO_TICKET` - Disables the use of session tickets for session resumption.
* `NO_SESSION_RESUMPTION_ON_RENEGOTIATION` - Always start a new session when performing a renegotiation on the server side.
* `NO_COMPRESSION` - Disables the use of TLS compression.
* `ALLOW_UNSAFE_LEGACY_RENEGOTIATION` - Allow legacy insecure renegotiation with servers or clients that do not support secure renegotiation.
* `SINGLE_ECDH_USE` - Creates a new key for each session when using ECDHE.  This is always enabled in OpenSSL 1.1.0.
* `SINGLE_DH_USE` - Creates a new key for each session when using DHE.  This is always enabled in OpenSSL 1.1.0.
* `TLS_ROLLBACK_BUG` - Disables version rollback attach detection.
* `NO_SSLV2` - Disables the use of SSLv2.
* `NO_SSLV3` - Disables the use of SSLv3.
* `NO_TLSV1` - Disables the use of TLSv1.0.
* `NO_TLSV1_1` - Disables the use of TLSv1.1.
* `NO_TLSV1_2` - Disables the use of TLSv1.2.
* `NO_TLSV1_3` - Disables the use of TLSv1.3.
* `NO_DTLSV1` - Disables the use of DTLSv1.0.
* `NO_DTLSV1_2` - Disables the use of DTLSv1.2.
* `NO_RENEGOTIATION` - Disallow all renegotiation in TLSv1.2 and earlier.
* `ENABLE_MIDDLEBOX_COMPAT` - Enable TLSv1.3 Compatibility mode.  Requires
  OpenSSL 1.1.1 or newer. This is on by default in 1.1.1, but a future version
  may have this disabled by default.

<!--
* `CIPHER_SERVER_PREFERENCE` - Use the server’s preferences rather than the
  client’s when selecting a cipher.  This has no effect on the client side;
  this option is included here for the sake of completeness.
* `PRIORITIZE_CHACHA` - Prioritize ChaCha ciphers when preferred by clients. Applies to server only
-->

## rustls_cipher_suites

{{since('dev')}}

Optional array of strings.

If set, then the value will be used to configure rustls cipher suites.

Rustls is used as described under the
[tls_prefer_openssl](#tls_prefer_openssl) option above.

The list of possible cipher suites at the time of writing is:

* `TLS13_AES_256_GCM_SHA384`
* `TLS13_AES_128_GCM_SHA256`
* `TLS13_CHACHA20_POLY1305_SHA256`
* `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`
* `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`
* `TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256`
* `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`
* `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`
* `TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256`

This list may evolve over time as new releases of kumomta are made.  You can
review the current possible list with the `tls-probe` utility:

```console
$ /opt/kumomta/sbin/tls-probe list-rustls-cipher-suites
TLS13_AES_256_GCM_SHA384
TLS13_AES_128_GCM_SHA256
TLS13_CHACHA20_POLY1305_SHA256
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
```

