# `kumo.on('tsa_init', FUNCTION)`

{{since('dev')}}

The `tsa_init` event is triggered once when the `tsa-daemon` process initializes.
The intent is that you use this event to define the database storage location and
listeners for your environment.

The event handler is not passed any parameters, and does not expect
any particular return value.

```lua
local tsa = require 'tsa'
local kumo = require 'kumo'

-- Called on startup to initialize the system
kumo.on('tsa_init', function()
  -- This is the default, so you needn't specify this.
  -- But if you wanted to change the path, you must do it
  -- before you start the listener
  tsa.configure_tsa_db_path '/var/spool/kumomta/tsa.db'

  tsa.start_http_listener {
    listen = '0.0.0.0:8008',
    -- allowed to access any http endpoint without additional auth
    -- You will likely want to include your LAN CIDR here if
    -- you are running multiple nodes
    trusted_hosts = { '127.0.0.1', '::1' },
  }
end)
```
