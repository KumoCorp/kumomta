---
description: "Monitor KumoMTA with Prometheus and Grafana — scrape kumod's own /metrics endpoint (no node_exporter needed) and import dashboard 21391."
---

# How Do I Monitor KumoMTA with Prometheus and Grafana?

KumoMTA exposes its own metrics endpoint. You scrape it directly; you do not need `node_exporter`.

```console
# Human-readable Prometheus format
$ curl -s 'http://localhost:8000/metrics'

# Same data as JSON
$ curl -s 'http://localhost:8000/metrics.json'
```

!!! note
    `node_exporter` reports host metrics (CPU, disk, memory) only. KumoMTA's delivery, queue, and connection metrics come from kumod's own `/metrics` endpoint on the HTTP listener port (default 8000). Point Prometheus at that.

Counters such as `total_messages_delivered` only appear once there has been activity since the last restart, so a freshly started node may not show them yet.

## Quick checks without Prometheus

```console
$ kcli top            # live TUI of top-line metrics
$ kcli queue-summary  # textual view of queue depths
```

## Prometheus + Grafana

Scrape each kumod node and import the starter dashboard (Grafana dashboard ID 21391):

```yaml
scrape_configs:
  - job_name: kumomta
    scrape_interval: 5s
    metrics_path: /metrics
    static_configs:
      - targets:
          - 'kumomta-1:8000'
          - 'kumomta-2:8000'
```

Access to `/metrics` is gated by the `trusted_hosts` setting on the HTTP listener. Add your Prometheus host there if it is remote.

## See also

* [Getting Server Status](../userguide/operation/status.md)
* [kumod Metrics](../reference/metrics/kumod/index.md)
