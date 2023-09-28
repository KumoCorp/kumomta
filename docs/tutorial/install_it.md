# Installing KumoMTA

Pre-built repos are available for supported Operating Systems, making installation straightforward:

```console
sudo dnf -y install dnf-plugins-core
sudo dnf config-manager \
    --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
sudo yum install kumomta
```

This installs the KumoMTA daemon to /opt/kumomta/sbin/kumod

!!!note
    Alternatively you can install the kumomta-dev package in order to take advantage of the latest pre-release features. This is only recommended for testing environments.

KumoMTA is now installed with a basic policy that allows relay from localhost, but it will need a more granular configuration policy for production use.

## Writing a Configuration Policy

The KumoMTA configuration is entirely written in [Lua](https://www.lua.org/home.html). Lua is a powerful embedded scripting language that is easy to read and code, and is very powerful. It is used for custom scripts in Cisco security appliances, Roblox, World of Warcraft, and really awesome MTAs. You can read more about how we leverage Lua [here](https://docs.kumomta.com/tutorial/lua_resources/).

1. Create an initial core configuration by copying the example at [https://docs.kumomta.com/userguide/configuration/example/](../userguide/configuration/example.md) and writing it to `/opt/kumomta/etc/policy/init.lua`.

1. Update the relay_hosts configuration within the start_esmtp_listener function to reflect which networks are authorized to inject mail:

    ```lua
    -- override the default set of relay hosts
    relay_hosts = { '127.0.0.1', '192.168.1.0/24' }
    ```

1. By default only localhost and private networks are able to relay (send) mail.  Add the IP address or CIDR block of your injectors here to allow them to relay mail.

    For HTTP, this is done with the _*trusted_hosts*_ setting in a listener stanza:

    ```lua
    kumo.start_http_listener {
      listen = '0.0.0.0:8000',
      -- allowed to access any http endpoint without additional auth
      trusted_hosts = { '127.0.0.1', '::1' },
    }
    ```

    !!!note
       If you are going to allow the HTTP listener on any IP other than localhost, you should also configure [TLS](https://docs.kumomta.com/reference/kumo/start_http_listener/?h=tls#tls_private_key) and [HTTP Validation](https://docs.kumomta.com/reference/events/http_server_validate_auth_basic/).

1. Copy the default Traffic Shaping helper configuration files into place. The helpers are designed to provide simple configuration for standard use cases:

    ```console
    sudo cp /opt/kumomta/share/policy-extras/shaping.toml /opt/kumomta/etc/
    ```

1. Configure the listener_domains.toml file, written to `/opt/kumomta/etc/listener_domains.toml` in the following format, substituting your own sending domain information:

    ```toml
    ["bounce.example.com"]
    # You can specify multiple options if you wish
    log_oob = true
    log_arf = true
    relay_to = false
    ```

    !!!note
        The preceding example configures the server to accept traffic from the outside world addressed to the bounce.example.com domain, as long as the incoming messages are either Out-Of-Band DSN (bounce) notifications, or Feedback Loop messages, but will not accept regular mail for inbound relay such as with a corporate mail environment.

1. Configure the sources.toml file, written to `/opt/kumomta/etc/sources.toml` in the following format, substituting your own IP and ehlo information:

    ```toml
    [source."ip-1"]
    source_address = "10.0.0.1"
    ehlo_domain = 'mta1.examplecorp.com'
    [source."ip-2"]
    source_address = "10.0.0.2"
    ehlo_domain = 'mta2.examplecorp.com'
    # Pool containing our two IPs, round-robin assigned with equal weighting
    [pool."Default"]
    [pool."Default"."ip-1"]
    [pool."Default"."ip-2"]
    ```

1. Configure the dkim_data.toml file, written to `/opt/kumomta/etc/dkim_data.toml` in the following format, substituting your own DKIM signing information:

    ```toml
    [base]
    # Default selector to assume if the domain/signature block
    # doesn't specify one
    selector = "dkim1024"

    # The default set of headers to sign if otherwise unspecified
    headers = ["From", "To", "Subject", "Date", "MIME-Version", "Content-Type", "Sender"]

    # Domain blocks match based on the sender domain of the incoming message
    [domain."example.com"]
    selector = 'dkim1024'
    headers = ["From", "To", "Subject", "Date", "MIME-Version", "Content-Type", "Sender"]
    algo = "sha256"

    # Optional override of keyfile path Default is "/opt/kumomta/etc/dkim/DOMAIN/SELECTOR.key"
    filename = "/full/path/to/key."
    ```

    !!!note
        These instructions assume that the keyfiles are already created and in place, along with the appropriate DNS records. See [the UserGuide](../userguide/configuration/dkim.md) for more information.

You now have a basic and safe sending configuration that will allow you to move on to the testing step.

## Starting KumoMTA

If you followed all the instructions above without errors, you should now have a working MTA on a properly sized server.

Check to see if it is already running from the install instructions above:

```bash
sudo systemctl status kumomta
```

If the service is already running, you will need to restart it to apply the changes made above:

```bash
sudo systemctl restart kumomta
```

Start the MTA with this:

```bash
sudo systemctl start kumomta
```

You can enable it to restart as a service with a reboot with:

```bash
sudo systemctl enable kumomta
```

Alternately you can start it manually with:

```bash
 sudo /opt/kumomta/sbin/kumod --policy \
 /opt/kumomta/etc/policy/init.lua --user kumod&
```

* Using sudo allows KumoMTA to run as a privileged user so it can access port 25 which is needed to deliver via SMTP to the internet.
* The daemon `kumod` is the MTA.
* The directive --policy makes kumod load the 'init.lua' file as configuration policy.
* Because we launched with sudo, you need to use the directive --user and provide a valid user to assign responsibility to.
* The line ends with a `&` that forces the daemon to run in the background and returns you to a usable prompt (use `fg` to bring it back to the foreground).

You can also get immediate feedback by pre-pending ```KUMOD_LOG=kumod=info``` (or debug for more detail):

```bash
sudo KUMOD_LOG=kumod=info /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod&
```

If all goes well, it should return a PID and drop you back to a Linux prompt.
