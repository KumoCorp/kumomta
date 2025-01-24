# Command-Line Help for `kcli`

This document contains the help content for the `kcli` command-line program.

## `kcli`

KumoMTA CLI.

Interacts with a KumoMTA instance via its HTTP API endpoint. To use it, you must be running an HTTP listener.

The default is to assume that KumoMTA is running a listener at http://127.0.0.1:8000 (which is in the default configuration), but otherwise you can override this either via the --endpoint parameter or KUMO_KCLI_ENDPOINT environment variable.

Full docs available at: <https://docs.kumomta.com>


**Usage:** `kcli [OPTIONS] <COMMAND>`

###### **Subcommands:**


* `bounce` — Administratively bounce messages in matching queues

* `bounce-list` — Returns list of current administrative bounce rules

* `bounce-cancel` — Cancels an admin bounce entry

* `rebind` — Rebind messages from matching queues into different queue(s)

* `suspend` — Administratively suspend messages in matching queues

* `suspend-list` — Returns list of current administrative suspend rules

* `suspend-cancel` — Cancels an admin suspend entry

* `suspend-ready-q` — Administratively suspend the ready queue for an egress path

* `suspend-ready-q-list` — Returns list of current ready queue/egress path suspend rules

* `suspend-ready-q-cancel` — Cancels an admin suspend entry for a ready queue/egress path

* `set-log-filter` — Changes the diagnostic log filter

* `inspect-message` — Returns information about a message in the spool

* `inspect-sched-q` — Returns information about a scheduled queue

* `provider-summary` — Prints a summary of the aggregate state of the queues from the perspective of the provider or destination site

* `queue-summary` — Prints a summary of the state of the queues, for a human to read

* `trace-smtp-client` — Trace outgoing sessions made by the SMTP service

* `trace-smtp-server` — Trace incoming connections made to the SMTP service

* `top` — Continually update and show what's happening in kumod

## Options


* `--endpoint <ENDPOINT>` — URL to reach the KumoMTA HTTP API. You may set KUMO_KCLI_ENDPOINT in the environment to specify this without explicitly using --endpoint. If not specified, http://127.0.0.1:8000 will be assumed





## Available Subcommands