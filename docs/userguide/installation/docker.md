# Installing KumoMTA in a Docker container

Our CI builds the latest version of our image and publishes it
to the GitHub Container registry:

* `ghcr.io/kumocorp/kumomta:latest` - the most recent stable release.
* `ghcr.io/kumocorp/kumomta-dev:latest` - the most recent dev release.

You'll need a policy script in order to start kumo.

Create a file named `init.lua` with these contents:

```lua
local kumo = require 'kumo'
-- This config acts as a sink that will discard all received mail

kumo.on('init', function()
  -- Listen on port 25
  kumo.start_esmtp_listener {
    listen = '0:25',
    -- allow all clients to send mail
    relay_hosts = { '0.0.0.0/0' },
  }

  -- Define the default "data" spool location.
  -- This is unused by this config, but we are required to
  -- define a default spool location.
  kumo.define_spool {
    name = 'data',
    path = '/tmp/kumo-sink/data',
  }

  -- Define the default "meta" spool location.
  -- This is unused by this config, but we are required to
  -- define a default spool location.
  kumo.define_spool {
    name = 'meta',
    path = '/tmp/kumo-sink/meta',
  }
end)

kumo.on('smtp_server_message_received', function(msg)
  -- Accept and discard all messages
  msg:set_meta('queue', 'null')
end)
```

When we launch the image, we want to mount our `init.lua` file into the image
and tell it to use it.  The default location for this is `/opt/kumomta/etc/policy`:

```console
$ sudo docker run --rm -p 2025:25 \
    -v .:/opt/kumomta/etc/policy \
    --name kumo-sink \
    ghcr.io/kumocorp/kumomta:latest
```

