---
description: "Why KumoMTA configuration changes seem to have no effect — what reloads automatically, what needs a restart, and how to bump the config epoch."
---

# Why Aren't My Configuration Changes Taking Effect?

KumoMTA caches configuration for performance, so changes are not always applied instantly. Knowing what reloads automatically, and what does not, avoids most "I changed it but nothing happened" confusion.

## What reloads automatically

* **Realtime events** (such as `get_queue_config` and `get_egress_path_config`) and the data files they read are re-evaluated as messages flow, subject to caching.
* The **Lua policy itself** is cached and refreshed every 300 seconds or 1024 executions by default.
* **Shaping** data refreshes on its configured epoch/TTL.
* **TLS certificates** are reloaded automatically on a short TTL (no restart needed).

Because of this caching there is no truly instantaneous reload. Expect up to a few minutes of latency.

## What requires an explicit reload or restart

Anything configured inside the **`init` event** is only read at startup, so changing it requires a reload. Listener settings applied at init (notably `start_esmtp_listener`, including its `trace_headers`) require a full restart to take effect.

## Forcing a refresh

To force configuration to be re-read without a full restart, bump the configuration epoch:

```console
$ curl -i 'http://localhost:8000/api/admin/bump-config-epoch' -X POST
```

If you need changes to propagate faster as a matter of course, lower the relevant TTLs.

## A common false alarm

A frequent cause of "my change had no effect" is editing a file the helper does not actually load. Confirm the path you expect is the one in use, then verify the result with `kcli queue-summary` / `kcli provider-summary`.

## See also

* [Configuration Concepts](../userguide/configuration/concepts.md)
* [POST /api/admin/bump-config-epoch](../reference/http/kumod/api_admin_bump_config_epoch_post.md)
