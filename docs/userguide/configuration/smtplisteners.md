# Configuring SMTP Listeners

An SMTP listener can be defined using the `kumo.start_esmtp_listener` function.

The `kumo.start_esmtp_listener` function can be called multiple times to define multiple listeners. Each listener can have its own relay list, banner, hostname and list of controls to determine domain behavior.

In the example below you can see the definition of IP address, Port, and specific relay hosts that are permitted to use that listener.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  hostname = 'mail.example.com',
  relay_hosts = { '127.0.0.1', '192.168.1.0/24' },
  ['send.example.com'] = {
      -- relay to anywhere, so long as the sender domain is send.example.com
      -- and the connected peer matches one of the listed CIDR blocks
      relay_from = { '10.0.0.0/24' },
    },
}
```

Refer to the [start_esmtp_listener](https://docs.kumomta.com/reference/kumo/start_esmtp_listener/) chapter of the Reference Manual for detailed options.
