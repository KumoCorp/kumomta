# `kumo.set_diagnostic_log_filter(FILTER)`

Changes the filtering configuration for the diagnostic log.

KumoMTA uses the *tracing-subscriber* Rust crate to provide diagnostic
logging.  This can be configured via the `KUMOD_LOG` environment to set
the logging for the process when it first starts up, but can also be
adjusted via your policy file by calling `kumo.set_diagnostic_log_filter`
if you wish.

The default log filter that is set at startup is `kumod=info` which causes all
informational logging from the `kumod` crate (the main KumoMTA server process)
to be logged.

If you want more verbose diagnostics, you might consider changing it from
inside your `init` event handler:

```lua
kumo.on('init', function()
  kumo.set_diagnostic_log_filter 'kumod=debug'
end)
```

The filter syntax is quite powerful, allowing you set different levels for
different crates.  The full set of filter directives are [explained
here](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives).
