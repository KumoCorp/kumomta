# `kumo.configure_log_hook {PARAMS}`

Configures the lua logging hook. When enabled, each matching log event will
cause cause new a [Message](../message/index.md) to generated and have its body
assigned to the log record (or to its template expansion if you have configured
that), and passed to the
[should_enqueue_log_record](../events/should_enqueue_log_record.md) event.

This allows you to utilize KumoMTA's internal queueing to dispatch log events
to external systems such as webhooks or some external queuing system for
processing elsewhere in your deployment architecture.

See [should_enqueue_log_record](../events/should_enqueue_log_record.md) for an example.

The following options are configurable for the logging hook and work the same
way as their counterparts in local log file logging. Rather than duplicate the
information here, this section links to those options:

* [back_pressure](configure_local_logs.md#back_pressure)
* [meta](configure_local_logs.md#meta)
* [headers](configure_local_logs.md#headers)
* [per_record](configure_local_logs.md#per_record)

In addition, the following options are supported:

## deferred_spool

If set to `true`, the generated message will not be immediately saved to the
spool in the case that your
[should_enqueue_log_record](../events/should_enqueue_log_record.md) indicates
that the message should be queued.
