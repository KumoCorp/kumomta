---
tags:
  - ops
  - debugging
---
# kcli trace-smtp-client


Trace outgoing sessions made by the SMTP service.

This is a diagnostic tool for the server operator.

Sessions are logged in real time.

Take care on a busy server with live traffic as this tracing mechanism will by-default match all traffic, but there is limited capacity/resources for tracing.  Outside of initial small scale testing, you will need to carefully consider filtering constraints in order to observe only the intended sessions, otherwise the tracing subsystem will be forced to drop a subset of trace events.

Filtering works by specifying an allow-list for specific properties of a trace session. If an allow-list for a given property is set, and the session has the corresponding property set, then the session is traced only if is value is contained in your set of allowed values.

Most session properties are filled out AFTER the session has been initiated, and a given session may attempt to establish a series of connections based on how the MX records are resolved, so you should anticipate seeing a number of session initiations that won't (yet) match your trace parameters.

The main session property that is known at initiation is the ready queue name, so if you know precisely the ready queue of interest, using `--ready-queue` will be the most focused and efficient filter you can specify.


**Usage:** `kcli trace-smtp-client [OPTIONS]`

## Options


* `--source <SOURCE>` — Add a source (in CIDR format) to the list of source addresses that we want to filter by. If any are specified, then only connections made from a matching address will be traced. If no sources are specified, any/all incoming SMTP connections will be traced.

     Can be used multiple times to add multiple candidate addresses.

     Eg: --source 10.0.0.1 --source 192.168.1.0/24

* `--mx-addr <MX_ADDR>` — Add an address (in CIDR format) to the list of MX host addresses that we want to filter by. If any are specified, then only connections made from a matching address will be traced. If no addresses are specified, any/all incoming SMTP connections will be traced.

     A given session may communicate with multiple MX addresses over its lifetime. The full list of MX addresses is not known at session initiation, and is filled in after they have been resolved.

     Can be used multiple times to add multiple candidate addresses.

     Eg: --mx-addr 10.0.0.1 --mx-addr 192.168.1.0/24

* `--mx-host <MX_HOST>` — The MX hostname to match. If omitted, any MX hostname will match!

     A given session may communicate with multiple MX addresses over its lifetime. The full list of MX addresses is not known at session initiation, and is filled in after they have been resolved.

* `--domain <DOMAIN>` — The domain name to match. If omitted, any domains will match!

     This is a per-message property, and is unavailable for matching until after a session has established a successful connection to a host and is ready to deliver a message. Until a message is present, this filter is ignored.

     A given connection in a session may transit messages with a variety of different domains.

* `--routing-domain <ROUTING_DOMAIN>` — The routing_domain name to match. If omitted, any routing domain will match!

     This is a per-message property, and is unavailable for matching until after a session has established a successful connection to a host and is ready to deliver a message. Until a message is present, this filter is ignored.

* `--campaign <CAMPAIGN>` — The campaign name to match. If omitted, any campaigns will match!

     This is a per-message property, and is unavailable for matching until after a session has established a successful connection to a host and is ready to deliver a message. Until a message is present, this filter is ignored.

     A given connection in a session may transit messages with a variety of different campaigns.

* `--tenant <TENANT>` — The tenant name to match. If omitted, any tenant will match!

     This is a per-message property, and is unavailable for matching until after a session has established a successful connection to a host and is ready to deliver a message. Until a message is present, this filter is ignored.

     A given connection in a session may transit messages with a variety of different tenants.

* `--egress-pool <EGRESS_POOL>` — The egress pool name to match. If omitted, any pool will match!

     This property is known at session initiation.

* `--egress-source <EGRESS_SOURCE>` — The egress source name to match. If omitted, any source will match!

     This property is known at session initiation.

* `--mail-from <MAIL_FROM>` — The envelope sender to match. If omitted, any will match.

     This is a per-message property, and is unavailable for matching until after a session has established a successful connection to a host and is ready to deliver a message. Until a message is present, this filter is ignored.

     A given connection in a session may transit messages with a variety of different envelopes.

* `--rcpt-to <RCPT_TO>` — The envelope recipient to match. If omitted, any will match.

     This is a per-message property, and is unavailable for matching until after a session has established a successful connection to a host and is ready to deliver a message. Until a message is present, this filter is ignored.

     A given connection in a session may transit messages with a variety of different envelopes.

* `--ready-queue <READY_QUEUE>` — The ready queue name to match. If omitted, any ready queue will match!

     This property is known at session initiation.

* `--color <COLOR>` — Whether to colorize the output

    Default value: `tty`

    Possible values: `tty`, `yes`, `no`


* `--only-new` — Trace only newly opened sessions; ignore data from previously opened sessions

* `--only-one` — Trace the first session that we observe, ignoring all others

* `--terse` — Abbreviate especially the write side of the transaction trace, which is useful when examining high traffic and/or large message transmission



