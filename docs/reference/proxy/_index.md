# proxy

The `proxy` module provides HTTP metrics and administration endpoints for the KumoProxy server.

```lua
local proxy = require 'proxy'
```

!!! note
    This module is only available to the `proxy-server` executable.

These functions should be called from inside your
[proxy_init](../events/proxy_init.md) event handler.

## Available Functions { data-search-exclude }
