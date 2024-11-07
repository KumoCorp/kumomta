# Using the kcli Command-Line Client

KumoMTA comes with several API endpoints to make administration of the server
easier for common tasks, and additionally comes with the **`kcli`**
command-line client which provides access to the APIs for local administrative
tasks.

The **`kcli`** client is located at `/opt/kumomta/sbin/kcli` and requires a
configured [HTTP Listener](../configuration/httplisteners.md) to function.

At minimum, the `kcli` client must be passed an endpoint and a command:

```console
$ kcli --endpoint http://127.0.0.1:8000 bounce-list
```

The `--endpoint` parameter indicates the HTTP API endpoint you have configured
for your KumoMTA instance.  This documentation uses `http://127.0.0.1:8000`
because that endpoint is shown as a suggested default for that http
listener.  You need to adjust it to match whatever you have configured for
your HTTP listener in your environment.

{{since('2023.08.22-4d895015', indent=True)}}
    The `--endpoint` defaults to `http://127.0.0.1:8000` and can be omitted.
    If `KUMO_KCLI_ENDPOINT` is set in the environment, its value will be used
    if `--endpoint` is not specified.

    In earlier versions, you need to explicitly pass `--endpoint`.

## Monitoring Queue Status

The `kcli` client can be used to report on messages currently
queued in the server with the following format:

```console
$ kcli --endpoint http://127.0.0.1:8000 queue-summary
{
SITE                                              SOURCE       PROTO       D T C   Q
(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com hdfc.s-ip102 smtp_client 0 0 5 995
(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com hdfc.s-ip103 smtp_client 0 0 5 995
...
}
```

!!! note
    This output format is subject to change and is not suitable for a machine to parse. It is expressly unstable and you must not depend upon it in automation.

    The data behind this output is pulled from the metrics.json endpoint, which is machine readable.

 The output is presented in two sections:

1. The ready queues
2. The scheduled queues

The ready queue data is presented in columns that are mostly self explanatory, but the numeric counts are labelled with single character labels:

* D - the total number of delivered messages
* T - the total number of transiently failed messages
* C - the number of open connections
* Q - the number of ready messages in the queue

Note that the ready queue counter values reset whenever the ready queue is reaped, which occurs within a few minutes of the ready queue being idle, so those numbers are only useful to get a sense of recent/current activity. Accurate accounting must be performed using the delivery logs and not via this utility.

The scheduled queue data is presented in two columns; the queue name and the number of messages in that queue.

## Managing Bounces

### Bouncing Messages

The `kcli` client can be used to administratively bounce messages currently
queued in the server with the following format:

```console
$ kcli --endpoint http://127.0.0.1:8000 bounce --everything --reason purge
{
  "id": "0234c7c9-afd3-49f9-9a4c-a1cc37fcc53b",
  "bounced": {},
  "total_bounced": 0
}
```

Allowed arguments for the bounce command include:

#### domain

Optional string. The domain name to match.
If omitted, any domain will match.

#### campaign

Optional string. The campaign name to match.
If omitted, any campaign will match.

#### tenant

Optional string. The tenant to match.
If omitted, any tenant will match.

!!! danger
    If you specify none of `domain`, `campaign` or `tenant`, then
    *ALL* queues will be bounced.

    With great power, comes great responsibility!

#### reason

Required. Reason to log in the delivery log.

#### duration

Optional duration string. Defaults to `"5m"`.
Specifies how long this bounce directive remains active.

While active, newly injected messages that match the
bounce criteria will also be bounced.

See the [Bounce API](../../reference/http/api_admin_bounce_v1.md) page of the
Reference Manual for more information.

### Listing Current Bounces

You can list the currently active bounce commands with the following command:

```console
$ kcli --endpoint http://127.0.0.1:8000 bounce-list
[
  {
    "id": "169c3dc0-6518-41ef-bfbb-1f0ae426cb32",
    "campaign": null,
    "tenant": null,
    "domain": null,
    "reason": "purge",
    "duration": "4m 50s 207ms 320us 231ns",
    "bounced": {
      "wezfurlong.org": 1
    },
    "total_bounced": 1
  }
]
```

See the [Admin Bounce List
API](../../reference/http/api_admin_bounce_list_v1.md) page of the Reference
Manual for more information.

### Removing a Bounce

Because bounce commands default to a five-minute duration, messages will
continue to bounce after the command has been issued. This helps with scenarios
such as when a campaign needs to be aborted, but the entire campaign may not
have been injected at the time the command was issued.

Sometimes after a bounce has been issued there is a need to cancel the bounce
before the time window has expired. Once a bounce command's ID is determined
with the `bounce-list` command, the bounce can be canceled with the
`bounce-cancel` command:

```console
$ kcli --endpoint http://127.0.0.1:8000 bounce-cancel --id 169c3dc0-6518-41ef-bfbb-1f0ae426cb32
removed 0234c7c9-afd3-49f9-9a4c-a1cc37fcc53b
```

See the [Bounce Cancel API](../../reference/http/api_admin_bounce_cancel_v1.md) page of the Reference Manual for more information.

## Managing Suspensions


## Setting The Diagnostic Log Level

While the log level is typically set in your configuration, it can also be set
on an ad-hoc basis using the set-log-filter command in `kcli`:

```console
$ kcli --endpoint http://127.0.0.1:8000 set-log-filter 'kumod=trace'
OK
```

See the [Set Diagnostic Log
Filter](../../reference/kumo/set_diagnostic_log_filter.md) page of the
Reference Manual for more information.

## Monitoring Inbound SMTP handshaking

When debugging, it is often helpful to monitor the full SMTP handshaking process in real-time.  The kcli client enables that for inbound connections with the `trace-smtp-server` function:

```console
$ kcli trace-smtp-server
```

Additional information on monitoring inbound connections is available on the [trace-smtp-server](../../reference/kcli/trace-smtp-server.md) page of the reference manual.

## Monitoring Outbound SMTP Connections

It is common to encounter issues when attempting to deliver to a given destination, while most destinations are delivered to without issues.

In those situations it helps to be able to monitor the oubound connections in question to identify any issues during the communications:

```console
$ kcli trace-smtp-client
```

Additional information on monitoring outbound connections is available on the [trace-smtp-client](../../reference/kcli/trace-smtp-client.md) page of the reference manual.
