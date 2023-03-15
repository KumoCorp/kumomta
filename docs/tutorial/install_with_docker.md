# Installing KumoMTA in a Docker container

## Quick Start with published Docker Image

Our CI builds the latest version of our image and publishes it
to the GitHub Container registry.

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
and tell it to use it.  The default location for this is `/opt/kumomta/policy`:

```console
$ sudo docker run --rm -p 2025:25 \
    -v .:/opt/kumomta/policy \
    --name kumo-sink \
    ghcr.io/kumomta/kumomta:main
```

# Longer version that shows how to setup docker

!!! todo
    Move most of this to a reference section

## Configure Docker

Ensure docker is actually installed in your server instance.

=== "DNF based systems"
    In Rocky, Alma, and any other DNF package manager system

    ```console
    $ sudo dnf config-manager --add-repo=https://download.docker.com/linux/centos/docker-ce.repo
    $ sudo dnf update -y
    $ sudo dnf install -y docker-ce docker-ce-cli containerd.io
    $ sudo systemctl enable docker
    ```

=== "APT based systems"

    In Ubuntu, Debian, and other Debial APT package management systems:

    ```console
    $ sudo apt update
    $ sudo apt install -y apt-utils docker.io
    $ sudo snap install docker
    ```

If you get an error that `/etc/rc.d/rc.local is not marked executable` then make it executable with `sudo chmod +x /etc/rc.d/rc.local`

### Start Docker

```console
$ sudo systemctl start docker
```

### Check if Docker is running

```console
$ systemctl status docker
```

### Enable Non-Root User Access

After completing Step 3, you can use Docker by prepending each command with sudo. To eliminate the need for administrative access authorization, set up a non-root user access by following the steps below.

1. Use the usermod command to add the user to the docker system group.
  ```console
  $ sudo usermod -aG docker $USER
  ```

2. Confirm the user is a member of the docker group by typing:
  ```console
  $ id $USER
  ```

It is a good idea to restart to make sure it is all set correctly.

## Build the KumoMTA container image

You need `git` to clone the repo:

=== "RPM based systems"
    ```console
    $ sudo dnf install -y git
    ```

=== "APT based systems"
    ```console
    $ sudo apt install -y git
    ```

Then clone the repo and run the image builder script:

```console
$ git clone https://github.com/kumomta/kumomta.git
$ cd kumomta
$ ./docker/kumod/build-docker-image.sh
```

This should result in something roughly like this:

```console
$ docker image ls kumomta/kumod
REPOSITORY      TAG       IMAGE ID       CREATED         SIZE
kumomta/kumod   latest    bbced15ff4d1   3 minutes ago   116MB
```

You can then run that image; this invocation mounts the kumo src dir at
`/config` and then the `KUMO_POLICY` environment variable is used to override
the default `/config/policy.lua` path to use the SMTP sink policy script
[sink.lua](https://github.com/kumomta/kumomta/blob/main/sink.lua), which will
accept and discard all mail:

```console
$ sudo docker run --rm -p 2025:25 \
    -v .:/config \
    --name kumo-sink \
    --env KUMO_POLICY="/config/sink.lua" \
    kumomta/kumod
```

