# per_record

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


