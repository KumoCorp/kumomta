# Check for success in the logs

When diagnosing the failure (or monitoring the success) of your test messages,
the logs provide extensive information.

The easiest way to monitor activity is with the built-in `tailer` utility. Open
a new terminal (because you cannot tail the logs and simultaneously send mail
from the same session) and start the `tailer` utility:

```bash
sudo /opt/kumomta/sbin/tailer --tail /var/log/kumomta
```

```json
{
  "type": "Reception",
  "id": "d7ef132b5d7711eea8c8000c29c33806",
  "sender": "test@example.com",
  "recipient": "test@example.com",
  "queue": "example.com",
  "site": "",
  "size": 320,
  "response": {
    "code": 250,
    "enhanced_code": null,
    "content": "",
    "command": null
  },
  "peer_address": {
    "name": "moto",
    "addr": "127.0.0.1"
  },
  "timestamp": 1695847980,
  "created": 1695847980,
  "num_attempts": 0,
  "bounce_classification": "Uncategorized",
  "egress_pool": null,
  "egress_source": null,
  "feedback_report": null,
  "meta": {},
  "headers": {},
  "delivery_protocol": null,
  "reception_protocol": "ESMTP",
  "nodeid": "d8e014c7-eaeb-4683-a56e-61324e91b1fc"
}
```

!!!note
    These example log entries have been formatted for ease of reading in the documentation.

This assumes a default installation with the logs located in `/var/log/kumomta/`.

If you want to dive in deeper, compressed logs are found in `/var/log/kumomta/` as can be seen in this tree. Logs are segmented by a combination of size/time and stored in compressed files named after the time that the segment was started. To read these, you need to unpack them first. You have [many options for configuring logging](../userguide/configuration/logging.md).

```bash
/var/log/kumomta
├── 20230311-033705
├── 20230311-033844
├── 20230312-182542
└── 20230314-181435
```

We can take a look at a specific log by decompressing it and since these are
zstd compressed, you can view all but the current one with `zstdcat`.  ZSTD is a streaming compression utility so the current file cannot be accessed until it is flushed and closed. You can force the current log to close early by stopping or restarting KumoMTA.

Using the example above, we can see the content of the newest file after
stopping KumoMTA with a `zstdcat /var/log/kumomta/20230314-181435`.

The default for log segments is to flush them after 1GB of data is written,
but you can adjust them to flush after a certain amount of time. The [kumo.configure_local_logs](../reference/kumo/configure_local_logs.md) page of the Reference Manual has all of the available logging configuration options.

## Next Steps

With KumoMTA installed, configured, and tested, the tutorial is complete.

See more on [Next Steps](./next_steps.md).
