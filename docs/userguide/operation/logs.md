# Viewing Logs

An important part of routine opperations is checking logs. KumoMTA compressed logs are found in /var/log/kumomta/ and are named by date stamp. Logs are segmented by a combination of size/time and stored in compressed files named after the time that the segment was started. To read these, you need to unpack them first. You have many options for configuring logging.

```console
/var/log/kumomta
├── 20230311-033705
├── 20230311-033844
├── 20230312-182542
└── 20230314-181435
```

We can take a look at a specific log by decompressing it and since these are [zstd compressed](https://github.com/facebook/zstd#readme), you can view all but the current one with zstdcat. ZSTD is a streaming compression utility so the current file cannot be accessed until it is flushed and closed. You can force the current log to close by stopping KumoMTA.

The default for log segments is to flush them after 1GB of data is written, but you can adjust them to flush after a certain amount of time. [kumo.configure_local_logs](https://docs.kumomta.com/reference/kumo/configure_local_logs/) has all of the available logging configuration options.

Using the example above, we can see the content of the newest file after stopping KumoMTA with a 'zstdcat /var/log/kumomta/20230314-181435'.

If you have not done so already, you will want to install `zstd` with a `(dnf or) apt install zstd'.  Below there is a sample of a decompressed received log:

`zstdcat /var/log/kumomta/20230428-201424_recv`
```console
{"type":"Reception","id":"44d70f50e60111ed8162000d3afc4acf","sender":"noreply@example.com",
"recipient":"recipient@example.com","queue":"example.com","site":"","size":27,
"response":{"code":250,"enhanced_code":null,"content":"","command":null},
"peer_address":{"name":"","addr":"127.0.0.1"},"timestamp":1682712864,"created":1682712864,
"num_attempts":0,"bounce_classification":"Uncategorized","egress_pool":null,"egress_source":null,
"feedback_report":null,"meta":{},"headers":{"Subject":"hello"}}
```
These JSON formatted logs can be programatically consumed or read manually as shown above for debugging and maintenance. [Formatting](https://docs.kumomta.com/userguide/configuration/logging/#customizing-the-log-format) can also be applied using the Mini Jinja tempating engine.
  


