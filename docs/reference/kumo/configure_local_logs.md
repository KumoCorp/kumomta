# `kumo.configure_local_logs {PARAMS}`

Enables local logging of reception and delivery events to the specified
`log_dir` directory.

Logs are written as zstd-compressed log file segments under the specified
directory.  Each line of the file is a JSON object holding information about
a reception or delivery related event.  The format of the Log Record object
can be found [here](../log_record.md).

This function should be called only from inside your [init](../events/init.md)
event handler.

```lua
kumo.on('init', function()
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs',
  }
end)
```

PARAMS is a lua table that can accept the keys listed below:

## back_pressure

Maximum number of outstanding items to be logged before
the submission will block; helps to avoid runaway issues
spiralling out of control.

```lua
kumo.configure_local_logs {
  -- ..
  back_pressure = 128000,
}
```

## compression_level

Specifies the level of *zstd* compression that should be used.  Compression
cannot be disabled.

Specifying `0` uses the zstd default compression level, which is `3` at the
time of writing.

Possible values are `1` (cheapest, lightest) through to `21`.

```lua
kumo.configure_local_logs {
  -- ..
  compression_level = 3,
}
```

## filter_event

{{since('2023.11.28-b5252a41')}}

Optional string. If provided, specifies the name of an event that should
be triggered to decide whether logs for a given message should be included
in this instance of local file logging.

The event will be passed the message that is being considered for logging
purposes.  The goal of the event is to return `true` if logging should
proceed or `false` otherwise.

You may access the message metadata to make that decision.

```lua
kumo.on('init', function()
  -- We only want logs for messages accepted via SMTP to land here
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs-smtp',
    filter_event = 'should_log_to_smtp_logs',
  }

  -- We want all other logs to land here
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs-other',
    filter_event = 'should_log_to_other',
  }
end)

kumo.on('should_log_to_smtp_logs', function(msg)
  return msg:get_meta 'reception_protocol' == 'ESMTP'
end)

kumo.on('should_log_to_other', function(msg)
  return msg:get_meta 'reception_protocol' ~= 'ESMTP'
end)
```

## headers

Specify a list of message headers to include in the logs. The default is
empty.

```lua
kumo.configure_local_logs {
  -- ..
  headers = { 'Subject' },
}
```

{{since('2023.12.28-63cde9c7', indent=True)}}
    Header names can now use simple wildcard suffixes; if the last character
    of the header name is `*` then it will match any string with that prefix.
    For example `"X-*"` will match any header names that start with `"X-"`.

## log_dir

Specifies the directory into which log file segments will be written.
This is a required key; there is no default value.

```lua
kumo.configure_local_logs {
  -- ..
  log_dir = '/var/log/kumo-logs',
}
```

## max_file_size

Specify how many uncompressed bytes to allow per file segment. When this number
is exceeded, the current segment is finished and a new segment is created.

Segments are created using the current time in the form `YYYYMMDD-HHMMSS` so that
it is easy to sort the segments in chronological order.

The default value is ~1GB of uncompressed data, which compresses down to around
50MB of data per segment with the default compression settings.

```lua
kumo.configure_local_logs {
  -- ..
  max_file_size = 1000000000,
}
```

## max_segment_duration

Specify the maximum time period for a file segment.  The default is unlimited.

If you set this to `"1min"`, you indicate that any given file should cover a
time period of 1 minute in duration; when that time period elapses, the current
file segment, if any, will be flushed and closed and any subsequent events will
cause a new file segment to be created.

```lua
kumo.configure_local_logs {
  -- ..
  max_segment_duration = '5 minutes',
}
```

## meta

Specify a list of message meta fields to include in the logs. The default is
empty.

```lua
kumo.configure_local_logs {
  -- ..
  meta = { 'my-meta-1' },
}
```

## per_record

Allows configuring per-record type logging.

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
      template = [[Bounce! id={{ id }}, from={{ sender }} code={{ response.code }} age={{ timestamp - created }}]],
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

The keys of the `per_record` table must correspond to one of the
record types listed below, or the special `Any` key which can be used
to match any record type that was not explicitly listed.  The values of
the `per_record` table are `LogRecordParams` have the following fields
and values:

* `suffix` - a string to append to the generated segment file name.
  For example, `suffix = '.csv'` will generate names like `20230306-022811.csv`.
* `log_dir` - specify an alternative log directory for this type
* `enable` - defaults to `true`. If you set it to `false`, records of this
  type will not be logged
* `segment_header` - ({{since('2023.11.28-b5252a41', inline=True)}}) text that will be written
  out to each newly opened segment file. Useful for emitting eg: a CSV header
  line.
* `template` - the template to use to format the log line. Continue reading
  below for more information.

The [Mini Jinja](https://docs.rs/minijinja/latest/minijinja/) templating engine
is used to evalute logging templates.  The full supported syntax is [documented
here](https://docs.rs/minijinja/latest/minijinja/syntax/index.html).

The JSON log record fields shown in the section below are assigned as template
variables, so using `{{ id }}` in your log template will be substituted with
the `id` field from the log record section below.

{% raw %}
To reference headers in a template, note that the header name is transformed to
lowercase as part of adding it to the `headers` object, so to access a header
named `X-FooBar` you would use `{{ headers['x-foobar'] }}` in your template.
{% endraw %}

{{since('2023.11.28-b5252a41', indent=True)}}
    You may now use `log_record` to reference the entire log record,
    which is useful if you want to replicate the default json representation
    of the log record for an individual record type.

    You might wish to use something like the following:

    {% raw %}
    ```lua
    per_record = {
        Feedback = {
            template = [[{{ log_record | tojson }}]]
        }
    }
    ```
    {% endraw %}

## min_free_space

{{since('dev')}}

Specifies the desired minimum amount of free disk space for the log storage
in this location.  Can be specified using either a string like `"10%"` to
indicate the percentage of available space, or a number to indicate the
number of available bytes.

If the available storage is below the specified amount then kumomta will
reject incoming SMTP and HTTP injection requests and the
[check-liveness](../rapidoc/#get-/api/check-liveness/v1) endpoint will indicate
that new messages cannot be received.

The default value for this option is `"10%"`.

## min_free_inodes

{{since('dev')}}

Specifies the desired minimum amount of free inodes for the log storage
in this location.  Can be specified using either a string like `"10%"` to
indicate the percentage of available inodes, or a number to indicate the
number of available inodes.

If the available inodes are below the specified amount then kumomta will
reject incoming SMTP and HTTP injection requests and the
[check-liveness](../rapidoc/#get-/api/check-liveness/v1) endpoint will indicate
that new messages cannot be received.

The default value for this option is `"10%"`.

## Log Record

The format of the Log Record object can be found [here](../log_record.md).

