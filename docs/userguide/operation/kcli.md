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


## Bouncing Messages

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

### domain

Optional string. The domain name to match.
If omitted, any domain will match.

### campaign

Optional string. The campaign name to match.
If omitted, any campaign will match.

### tenant

Optional string. The tenant to match.
If omitted, any tenant will match.

!!! danger
    If you specify none of `domain`, `campaign` or `tenant`, then
    *ALL* queues will be bounced.

    With great power, comes great responsibility!

### reason

Required. Reason to log in the delivery log.

### duration

Optional duration string. Defaults to `"5m"`.
Specifies how long this bounce directive remains active.

While active, newly injected messages that match the
bounce criteria will also be bounced.

See the [Bounce API](../../reference/http/api_admin_bounce_v1.md) page of the
Reference Manual for more information.

## Listing Current Bounces

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

## Removing a Bounce

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

## Monitoring inbound SMTP handshaking 

When debugging, it is often helpful to monitor the full SMTP handshaking process in real-time.  The kcli client enables that for inbound connections with the `trace-smtp-server` function:

```console
$ kcli trace-smtp-server
```



