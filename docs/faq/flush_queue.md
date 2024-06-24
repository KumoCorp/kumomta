# How do I flush a queue?

{{since('dev')}}

You can use `kcli rebind` for this; the following will flush `example.com` for all
tenant/campaigns:

```console
$ kcli rebind --domain example.com --always-flush
```

`kcli rebind` re-evaluates the queue for messages in matching queues. In the
usage above, we're not passing in any new metadata, so the queue won't actually
change. The `--always-flush` parameter tells KumoMTA that it should make the
messages immediately eligible for delivery even though we didn't change the
queue.

If you want to do this via API, then 
you should look at the
[/api/admin/rebind/v1](../../reference/rapidoc/#post-/api/admin/rebind/v1) HTTP endpoint
documentation.
