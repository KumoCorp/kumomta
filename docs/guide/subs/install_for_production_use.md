# Installing KumoMTA for Production Use

If you plan to use KumoMTA for production use without modification, you can install a docker image and "just run it".

To build a lightweight alpine-based docker image:

- Prepare your system with the needed essentials
- Ensure docker is actually installed in your server instance.

## APT based systems

In Ubuntu, Debian, and other Debial APT package management systems:

```bash
sudo apt update
sudo apt install -y apt-utils docker.io
sudo snap install docker
```

## DNF based systems

In Rocky, Alma, and any other DNF package manager system

```bash 
 sudo dnf config-manager --add-repo=https://download.docker.com/linux/centos/docker-ce.repo
 sudo dnf update -y
 sudo dnf install -y docker-ce docker-ce-cli containerd.io
 sudo systemctl enable docker
 ```

If you get an error that `/etc/rc.d/rc.local is not marked executable` then make it executable with `sudo chmod +x /etc/rc.d/rc.local`

## Special Case for CentOS7

First prepare your system by making sure it has the most current updates, includes wget, and any testing tools you need like telnet and curl.

To run KumoMTA in Centos7, download the prebuilt RPM and policy.

RPM: [https://github.com/kumomta/kumomta/suites/11445755838/artifacts/590348846](https://github.com/kumomta/kumomta/suites/11445755838/artifacts/590348846)
  
Simple policy: [https://github.com/kumomta/kumomta/blob/main/simple_policy.lua](https://github.com/kumomta/kumomta/blob/main/simple_policy.lua)
  
Sink policy: [https://github.com/kumomta/kumomta/blob/main/sink.lua](https://github.com/kumomta/kumomta/blob/main/sink.lua)
  
You should `unzip centos7.zip`
  
Then install with `rpm -ivh centos7/kumomta-2023.03.08_b3fa0dab-1.centos7.x86_64`
  
This will install a working copy of KumoMTA at `/usr/bin/kumod`
 
You can pull a copy of the simple_policy.lua or sink.lua and then run it like:

`/usr/bin/kumod --policy simple_policy.lua`
  
  **OR**
  
Follow this to do it from the command line:

```bash
# Prepare the system first
sudo yum install -y dnf
sudo dnf clean all
sudo dnf update -y
sudo dnf install -y libxml2 libxml2-devel clang curl telnet git bzip2 wget openssl-devel

# Now install KumoMTA
cd
sudo wget https://github.com/kumomta/kumomta/suites/11445755838/artifacts/590348846
sudo wget https://github.com/kumomta/kumomta/blob/main/simple_policy.lua
sudo wget https://github.com/kumomta/kumomta/blob/main/sink.lua
sudo unzip centos7.zip
rpm -ivh centos7/kumomta-2023.03.08_b3fa0dab-1.centos7.x86_64.rpm
sudo /usr/bin/kumod --policy sink.lua --user $USER
```

CentOS7 users can disregard the rest of this page.
  
### Start Docker

`sudo systemctl start docker`

### Check if Docker is running

`systemctl status docker`

### Enable Non-Root User Access

After completing Step 3, you can use Docker by prepending each command with sudo. To eliminate the need for administrative access authorization, set up a non-root user access by following the steps below.

1. Use the usermod command to add the user to the docker system group.
  `sudo usermod -aG docker $USER`

2. Confirm the user is a member of the docker group by typing:
  `id $USER`

It is a good idea to restart to make sure it is all set correctly (init 6)

### Build the docker image

At the time of this writing, the Docker image needs to be built from the project repo.  You will need to clone the repo and then build the image from `./docker`.

```bash
sudo dnf install -y git
git clone https://github.com/kumomta/kumomta.git

cd kumomta/docker/redis/
sudo docker build -t redis-cell .
docker run --name redis -p 6379:6379 -d redis-cell
cd ../..
sudo ./docker/kumod/build-docker-image.sh
```

This should result in something roughly like this:

```bash
docker image ls kumomta/kumod
REPOSITORY      TAG       IMAGE ID       CREATED         SIZE
kumomta/kumod   latest    bbced15ff4d1   3 minutes ago   116MB
```

You can then run that image; this invocation mounts the kumo src dir at `/config` and then the `KUMO_POLICY` environment
variable is used to override the default `/config/policy.lua` path to use the SMTP sink policy script [sink.lua](https://github.com/kumomta/kumomta/blob/main/sink.lua), which will accept and discard all mail:

```bash
$ sudo docker run --rm -p 2025:25 \
    -v .:/config \
    --name kumo-sink \
    --env KUMO_POLICY="/config/sink.lua" \
    kumomta/kumod
```
