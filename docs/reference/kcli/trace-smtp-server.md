---
tags:
  - ops
  - debugging
---
# kcli trace-smtp-server


Trace incoming connections made to the SMTP service.

This is a diagnostic tool for the server operator.

Connections are logged in real time.

Take care to use an appropriate `--source` when using this with a live busy server, as you will be overwhelmed by the traffic.


**Usage:** `kcli trace-smtp-server [OPTIONS]`

## Options


* `--source <SOURCE>` — Add a source (in CIDR format) to the list of source addresses that we want to filter by. If any are specified, then only connections made from a matching address will be traced. If no sources are specified, any/all incoming SMTP connections will be traced.

     Can be used multiple times to add multiple candidate addresses.

     Eg: --source 10.0.0.1 --source 192.168.1.0/24

* `--color <COLOR>` — Whether to colorize the output

    Default value: `tty`

    Possible values: `tty`, `yes`, `no`


* `--only-new` — Trace only newly opened sessions; ignore data from previously opened sessions

* `--only-one` — Trace the first session that we observe, ignoring all others

* `--terse` — Abbreviate especially the read side of the transaction trace, which is useful when examining high traffic and/or large message transmission



