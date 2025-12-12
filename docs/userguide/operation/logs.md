# Viewing Logs

An important part of routine opperations is checking logs. KumoMTA compressed
logs are found in `/var/log/kumomta/` by default and are named by date stamp.
Logs are segmented by a combination of size/time and stored in compressed
files named after the time that the segment was started. To read these, you
need to unpack them first. You have many options for configuring logging.

If you prefer to spread segments across a directory hierarchy, configure
[`log_dir`](../../reference/kumo/configure_local_logs/log_dir.md) to include
[`strftime`](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)
placeholders. For example, a configuration such as:

```lua
kumo.configure_local_logs {
  log_dir = "/var/log/kumo-logs/%Y/%m/%d",
}
```

stores new segments under directories like `/var/log/kumo-logs/2025/11/15`.
Dynamic paths are resolved for every write using UTC by default, and the
required directories are created automatically as the timestamps roll forward
(hourly, daily, and so on).

Time zone behavior:

* Directory templates use **UTC** by default so the generated paths align across
  hosts. Set `log_dir_timezone = 'Local'` (globally or per record) if you want
  directories to match the server's `date` output instead.
* The JSON log records themselves keep their timestamps in **UTC** (via
  `chrono::Utc::now()`), matching the [`kumo.time/Time` API](../../reference/kumo.time/Time.md)
  which stores wall-clock values in UTC internally. Using local or UTC for
  paths and UTC inside the record payloads is intentional and does not affect
  log creation or rotation.

```
/var/log/kumomta
├── 20230311-033705
├── 20230311-033844
├── 20230312-182542
└── 20230314-181435
```

## Using `tailer`

The `tailer` utility, found at `/opt/kumomta/sbin/tailer`, is the easiest way to quickly
review logs.  The `--tail` flag will follow the log files in real time:

```console
$ sudo /opt/kumomta/sbin/tailer --tail /var/log/kumomta
/var/log/kumomta/20230809-153944
{"type":"Reception","id":"f68462cf36ca11ee99f450ebf67f93bd","sender":"wez@exa
mple.com","recipient":"wez@example.com.org","queue":"example.com.org","site":
"","size":518,"response":{"code":250,"enhanced_code":null,"content":"","comma
nd":null},"peer_address":{"name":"foo.lan","addr":"127.0.0.1"},"timestamp":16
91595584,"created":1691595584,"num_attempts":0,"bounce_classification":"Uncat
egorized","egress_pool":null,"egress_source":null,"feedback_report":null,"met
a":{},"headers":{},"delivery_protocol":null,"reception_protocol":"ESMTP","nod
eid":"4eb22880-dc03-44dc-a4d1-4d0e68ac7845"}
waiting for more files
```

The above example is shown artificially wrapped for the purposes of displaying
nicely in this documentation. The actual log records are not output with wrapping.

## Manually

We can take a look at a specific log by decompressing it and since these are
[zstd compressed](https://github.com/facebook/zstd#readme), you can view all
but the current one with zstdcat. ZSTD is a streaming compression utility so
the current file cannot be accessed until it is flushed and closed. You can
force the current log to close by stopping KumoMTA.

The default for log segments is to flush them after 1GB of data is written, but
you can adjust them to flush after a certain amount of time if you find that
you are regularly wanting to inspect the logs on a live system.

[kumo.configure_local_logs](../../reference/kumo/configure_local_logs/index.md)
has all of the available logging configuration options.

Using the example above, we can see the content of the newest file after
stopping KumoMTA with a `zstdcat /var/log/kumomta/20230314-181435`.

If you have not done so already, you will want to install `zstd` with a (`dnf`
or) `apt install zstd`.  Below there is a sample of a decompressed received log:

```console
$ zstdcat /var/log/kumomta/20230428-201424_recv
{"type":"Reception","id":"44d70f50e60111ed8162000d3afc4acf","sender":"noreply@example.com",
"recipient":"recipient@example.com","queue":"example.com","site":"","size":27,
"response":{"code":250,"enhanced_code":null,"content":"","command":null},
"peer_address":{"name":"","addr":"127.0.0.1"},"timestamp":1682712864,"created":1682712864,
"num_attempts":0,"bounce_classification":"Uncategorized","egress_pool":null,"egress_source":null,
"feedback_report":null,"meta":{},"headers":{"Subject":"hello"}}
```

These JSON formatted logs can be programatically consumed or read manually as
shown above for debugging and maintenance.

[Formatting](../configuration/logging.md#customizing-the-log-format)
can also be applied using the Mini Jinja templating engine.

