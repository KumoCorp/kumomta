# Deploying Traffic Shaping Automation

The `tsa-daemon` process communicates with KumoMTA nodes to process tempfail and permfail events and issue commands to the KumoMTA nodes based on those events.

When running in a clustered environment each node needs to talk to each `tsa-daemon` process running in the cluster.

This can either be architected as one daemon per node or one or more daemons common to the cluster (see the [Deployment Architecture](./deployment.md) page).

When configuring clustered Traffic Shaping Automation, the steps are similar to what is covered in the [Configuring Traffic Shaping Automation](../configuration/trafficshapingautomation.md) page, but with some minor modifications.

1. In the `tsa_init.lua` file, the `tsa_init` handler must use a `trusted_hosts` list that includes all nodes in the cluster:

    ```lua
    kumo.on('tsa_init', function()
      tsa.start_http_listener {
        listen = '0.0.0.0:8008',
        trusted_hosts = { '127.0.0.1', '192.168.1.0/24', '::1' },
      }
    end)
    ```

2. In the `init.lua` file, the call to `shaping:setup_with_automation` must be modified to include publishing to all the TSA daemon instances:

    ```lua
    local shaper = shaping:setup_with_automation {
        publish = { 'http://127.0.0.1:8008', 'http://192.168.1.10:8008' },
        subscribe = { 'http://127.0.0.1:8008' },
        extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },
    }
    ```
