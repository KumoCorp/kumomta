# Configuring Logging

By default, KumoMTA writes to a JSON log format, the format of which can be found on the [Logging Reference Page](../../reference/kumo/configure_local_logs.md#log-record),

## Basic Log Configuration

The simplest logging configuration, added to the init event, is as follows:

```lua
kumo.configure_local_logs {
  log_dir = '/var/log/kumomta',
}
```

For multiple log files, the `configure_local_logs` function can be called multiple times with different parameters.

!!!note
    Logs can also be published as webhooks. See the [Publishing Log Events Via Webhooks](../operation/webhooks.md) chapter.

## OS Considerations

The log directory should be isolated to its own partition, in order to prevent a full log partition from affecting the overall server. For best performance, the log directory should be on a separate disk from the spool. The log partition should be monitored to ensure that the disk does not fill to 100% capacity.

## Compression and Rotation

By default, the server writes logs in a [Zstandard](https://en.wikipedia.org/wiki/Zstd) **zstd** compressed format, resulting in high storage efficiency as logs are written, instead of having to write large files to disk and compress them during file rotation.

By default, files are rotated after every 1Gb of uncompressed log data, resulting in files on disk that are approximately 50MB in size. The maximum size is configurable, see the [Logging Reference Page](../../reference/kumo/configure_local_logs.md#max_file_size). Rotated logs can be viewed using the *ztsdcat* utility. It is possible to configure KumoMTA to rotate logs on a time interval basis:

```lua
kumo.configure_local_logs {
  -- ..
  max_segment_duration = '5 minutes',
}
```

## Logging Message Headers

It's a common practice to encode important per-user or per-campaign information in message headers, or to use a message Subject line as an identifier in reporting. This requires logging the headers, which can be achieved by specifying the desired headers in the configuration:

```lua
kumo.configure_local_logs {
  -- ..
  headers = { 'Subject', 'X-Client-ID' },
}
```

## Customizing the log format

If a non-JSON format is needed for the logs, the *template* option can be used:

{% raw %}
```lua
kumo.configure_local_logs {
  -- ..
  template = [[{{type}} id={{ id }}, from={{ sender }} code={{ code }} age={{ timestamp - created }}]],
}
```
{% endraw %}

The [Mini Jinja](https://docs.rs/minijinja/latest/minijinja/) templating engine
is used to evalute logging templates.  The full supported syntax is [documented
here](https://docs.rs/minijinja/latest/minijinja/syntax/index.html). Any key present in the [default log format](../../reference/kumo/configure_local_logs.md#log-record) can be used in the templating engine.

## Configuring Individual Record Types

Sometimes it is necessary to configure logging on a more granular basis, especially when using custom log formats. KumoMTA supports this using the *per_record* option:

{% raw %}
```lua
kumo.configure_local_logs {
  per_record = {
    Reception = {
      -- use names like "20230306-022811_recv" for reception logs
      suffix = '_recv',
    },

    Delivery = {
      -- put delivery logs in a different directory
      log_dir = '/var/log/kumo/delivery',
    },

    TransientFailure = {
      -- Don't log transient failures
      enable = false,
    },

    Bounce = {
      -- Instead of logging the json record, evaluate this
      -- template string and log the result.
      template = [[Bounce! id={{ id }}, from={{ sender }} code={{ code }} age={{ timestamp - created }}]],
    },

    -- For any record type not explicitly listed, apply these settings.
    -- This effectively turns off all other log records
    Any = {
      enable = false,
    },
  },
}
```
{% endraw %}

This can be used to override paths, disable logs, or customize the format of specific event types.
