# Configuring SMTP Listeners

An SMTP listener can be defined using the `kumo.start_esmtp_listener` function.

The `kumo.start_esmtp_listener` function can be called multiple times to define multiple listeners. Each listener can have its own relay list, banner, hostname and list of controls to determine domain behavior.

In the example below you can see the definition of IP address, Port, and specific relay hosts that are permitted to use that listener.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  hostname = 'mail.example.com',
  relay_hosts = { '127.0.0.1', '192.168.1.0/24' },
}
```

Refer to the [start_esmtp_listener](https://docs.kumomta.com/reference/kumo/start_esmtp_listener/) chapter of the Reference Manual for detailed options.

For most use cases, it will be necessary to configure listeners on a per-domain basis regarding inbound traffic. This includes designating which domains are allowed for inbound relay and bounce/feedback loop processing. See the [Configuring Inbound and Relay Domains](./domains.md) section of the User Guide for more information.

## Securing Inbound SMTP Listeners with SMTP AUTH

While the `relay_hosts` option is often sufficient when receiving mail from internal systems, those environments that receive messages from external hosts should considering implementing SMTP AUTH authentication using username/password.

For more information, see the [Checking Inbound SMTP Authentication](../policy/auth.md) page.
