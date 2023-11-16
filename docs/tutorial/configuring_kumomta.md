# Configuring KumoMTA

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

    ```bash
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

1. Configure DKIM signing keys. [Read the guide](https://docs.kumomta.com/userguide/configuration/dkim/) for details, but the short version is below:

    Replace the domain and selector with your own, then generate signing keys with:

    ```bash
    export DOMAIN=<your_domain>
    export SELECTOR=<your_selector>
    sudo mkdir -p /opt/kumomta/etc/dkim/$DOMAIN
    sudo openssl genrsa -f4 -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.key 1024
    sudo openssl rsa -in /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.key -outform PEM -pubout -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.pub
    sudo chown kumod:kumod /opt/kumomta/etc/dkim/$DOMAIN -R
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

You now have a basic and safe sending configuration that will allow you to move on to [Starting KumoMTA](./starting_kumomta.md).
