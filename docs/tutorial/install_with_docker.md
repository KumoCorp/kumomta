# Installing KumoMTA in a Docker container

We are not currently publishing our docker image to a public registry,
so you will need to build the image for yourself.

## Configure Docker

Ensure docker is actually installed in your server instance.

=== "DNF based systems"
    In Rocky, Alma, and any other DNF package manager system

    ```bash
    sudo dnf config-manager --add-repo=https://download.docker.com/linux/centos/docker-ce.repo
    sudo dnf update -y
    sudo dnf install -y docker-ce docker-ce-cli containerd.io
    sudo systemctl enable docker
    ```

=== "APT based systems"

    In Ubuntu, Debian, and other Debial APT package management systems:

    ```bash
    sudo apt update
    sudo apt install -y apt-utils docker.io
    sudo snap install docker
    ```

If you get an error that `/etc/rc.d/rc.local is not marked executable` then make it executable with `sudo chmod +x /etc/rc.d/rc.local`

### Start Docker

```bash
$ sudo systemctl start docker
```

### Check if Docker is running

```bash
$ systemctl status docker
```

### Enable Non-Root User Access

After completing Step 3, you can use Docker by prepending each command with sudo. To eliminate the need for administrative access authorization, set up a non-root user access by following the steps below.

1. Use the usermod command to add the user to the docker system group.
  ```baseh
  $ sudo usermod -aG docker $USER
  ```

2. Confirm the user is a member of the docker group by typing:
  ```bash
  $ id $USER
  ```

It is a good idea to restart to make sure it is all set correctly.

## Build the KumoMTA container image

You need `git` to clone the repo:

=== "RPM based systems"
    ```bash
    $ sudo dnf install -y git
    ```

=== "APT based systems"
    ```bash
    $ sudo apt install -y git
    ```

Then clone the repo and run the image builder script:

```bash
$ git clone https://github.com/kumomta/kumomta.git
$ cd kumomta
$ ./docker/kumod/build-docker-image.sh
```

This should result in something roughly like this:

```bash
$ docker image ls kumomta/kumod
REPOSITORY      TAG       IMAGE ID       CREATED         SIZE
kumomta/kumod   latest    bbced15ff4d1   3 minutes ago   116MB
```

You can then run that image; this invocation mounts the kumo src dir at
`/config` and then the `KUMO_POLICY` environment variable is used to override
the default `/config/policy.lua` path to use the SMTP sink policy script
[sink.lua](https://github.com/kumomta/kumomta/blob/main/sink.lua), which will
accept and discard all mail:

```bash
$ sudo docker run --rm -p 2025:25 \
    -v .:/config \
    --name kumo-sink \
    --env KUMO_POLICY="/config/sink.lua" \
    kumomta/kumod
```

