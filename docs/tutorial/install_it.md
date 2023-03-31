# Installing It

Congratulations for making it this far. Now we can actually install some software.  This is probably the easiest part.

In another section of the documentation we show how to build from source but we are not covering that in this tutorial.  Here, we are going to take the easy path and install the [prebuilt binary from the repo](https://docs.kumomta.com/userguide/installation/getting_started/).  You can literally just copy/paste the commands below to install it from our yum repo.

```console
sudo dnf -y install dnf-plugins-core
sudo dnf config-manager \
    --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
sudo yum install kumomta-dev
```

This installs the KumoMTA daemon to /opt/kumomta/sbin/kumod 

Technically KumoMTA is now installed, but it will need a configuration policy in order to do anything useful. 


### Writing Config Policy
The KumoMTA configuration is entirely written in [Lua](https://www.lua.org/home.html).  If you have not heard of Lua before, that is ok, you are not alone.  It is a powerful scripting language that is easy to read and code, but is very powerful.  It is used for custom scripts in Cisco security appliances, Roblox, World of Warcraft, and really awesome MTAs. You can read more about how we leverage Lua [here](https://docs.kumomta.com/tutorial/lua_resources/).

To save you from writing your own policy from scratch, you can just copy the example found in the [User Guide](https://docs.kumomta.com/userguide/installation/getting_started/) and modify the relevant sections.  Create that in ```/opt/kumomta/etc/policy/init.lua``` like this:

```console
sudo vi /opt/kumomta/etc/policy/init.lua
```
... and paste the example configuration.  You can then edit the config to adjust things like outbound port, queues, banners, etc.


For instance, you can change the IP address you want to egress from in the 'define_egress_source' section of config.

```console
kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
  }
end)
```

You can also make sure only authorized systems can access your MTA.  For SMTP, this is done in the configuration with relay_hosts:

```lua
-- override the default set of relay hosts
relay_hosts = { '127.0.0.1', '192.168.1.0/24' }
```
By default only localhost and private networks are able to relay (send) mail.  Add the IP address or CIDR block of your injectors here to allow them to relay mail.

For HTTP, this is done with the _*trusted_hosts*_ setting in a listener stanza.
```lua
kumo.start_http_listener {
  listen = '0.0.0.0:8000',
  -- allowed to access any http endpoint without additional auth
  trusted_hosts = { '127.0.0.1', '::1' },
}
```


That will provide you with a basic and safe sending configuration that will allow you to move on to the testing step - we can examine the details later.

## Start it up


If you followed all the instructions above without errors, you should now have a working MTA on a properly sized server.  Lets test that theory.

Start the MTA with this:
```
sudo systemctl start kumomta
```
You can enable it to restart as a service with a reboot with:
```
sudo systemctl enable kumomta
```

Alternately you can start it manually with:
```console
 sudo /opt/kumomta/sbin/kumod --policy \ 
 /opt/kumomta/etc/policy/init.lua --user kumod&
```

 * Using sudo allows it to run as a privileged user so it can access port 25 which is needed to deliver via SMTP to the internet.
 * The daemon `kumod` is the MTA
 * The directive --policy makes kumod load the 'init.lua' file as configuration policy.
 * Because we launched with sudo, you need to use the directive --user and provide a valid user to assign responsibility to.
 * The line ends with a `&` that forces the daemon to run in the background and returns you to a usable prompt (use `fg` to bring it back to the foreground)

You can also get immediate feedback by pre-pending ```KUMOD_LOG=kumod=info``` (or debug for more detail) like this:
```console
sudo KUMOD_LOG=kumod=info /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod&
```

If all goes well, it should return a PID and drop you back to a Linux prompt.

