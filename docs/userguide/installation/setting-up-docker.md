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
