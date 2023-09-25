# Check for success in the logs

Regardless of whether the mail delivers or not, you should take a look at the
logs. The easiest way to see what is going on iw with the built-in tailer utility.  Simply open a new terminal (so you can work on other things and watch logs at the same time) and run this:
```bash
sudo /opt/kumomta/sbin/tailer --tail /var/log/kumomta
``` 
This assumes a default installation with the logs located in /var/log/kumomta/.

If you want to dive in deeper, compressed logs are found in `/var/log/kumomta/` as can be seen in this
tree. Logs are segmented by a combination of size/time and stored in compressed
files named after the time that the segment was started. To read these, you
need to unpack them first. You have [many options for configuring
logging](../userguide/configuration/logging.md).

```info
/var/log/kumomta
├── 20230311-033705
├── 20230311-033844
├── 20230312-182542
└── 20230314-181435
```

We can take a look at a specific log by decompressing it and since these are
zstd compressed, you can view all but the current one with `zstdcat`.  ZSTD is a
streaming compression utility so the current file cannot be accessed until it
is flushed and closed.  You can force the current log to close by stopping KumoMTA.

Using the example above, we can see the content of the newest file after
stopping KumoMTA with a `zstdcat /var/log/kumomta/20230314-181435`.

The default for log segments is to flush them after 1GB of data is written,
but you can adjust them to flush after a certain amount of time.
[kumo.configure_local_logs](../reference/kumo/configure_local_logs.md) has
all of the available logging configuration options.

