# Check for success in the logs

Reguardless of whether the mail delivers or not, you should take a look at the logs.  Compressed logs are found in ```/var/log/kumomta/``` as can be seen in this tree. Logs are bundled by day and compressed, so to read these, you need to unpack them first. The logs are typically stored in date formatted files, but you have [many options](https://docs.kumomta.com/userguide/configuration/logging/) and KumoMTA is highly configurable.

```info
/var/log/kumomta
├── 20230311-033705
├── 20230311-033844
├── 20230312-182542
└── 20230314-181435
```

We can take a look at a specific log by decompressing it and since these are zstd compressed, you can view all but the current one with zstdcat.  ZSTD is a streaming compression utility so the current file cannot be accessed until it is closed.  You can force the current log to close by stopping KumoMTA.

Using the example above, we can see the content of the newest file after stopping KumoMTA with a zstdcat /var/log/kumomta/20230314-181435.

While this may seem cumbersome, this method is only use for initial debugging.  A full implemementation of RabbitMQ is normally employed so that production logging events are streamed in real-time to your external logging database.

Logging is a complex topic and should be reviewd [here](https://docs.kumomta.com/reference/kumo/configure_local_logs/)

