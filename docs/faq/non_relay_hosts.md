# Why is KumoMTA Accepting Connections From Systems Not Listed in relay_hosts?

Let's say you have the following configured for your listeners:

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  relay_hosts = { '127.0.0.1', '192.168.1.0/24' },
}
```

And the following configured for your `listener_domains`` helper:

{% call toml_data() %}
["*"]
relay_to = false
log_oob = true
log_arf = true
{% endcall %}

As you operate your server, you may be concerned to discover in your logs that messages are bring accepted from hosts not listed in your `relay_hosts` list.

```json
{
  "type": "Reception",
  "id": "ebaaab5e9e7b11eeb206960002ccea16",
  "sender": "test@test.com",
  "recipient": "mmiihhww09@gmail.com",
  "queue": "default@gmail.com",
  "site": "",
  "size": 594,
  "response": {
    "code": 250,
    "enhanced_code": null,
    "content": "",
    "command": null
  },
  "peer_address": {
    "name": "WIN-CLJ1B0GQ6JP",
    "addr": "91.92.251.215"
  },
  "timestamp": 1702996556,
  "created": 1702996556,
  "num_attempts": 0,
  "bounce_classification": "Uncategorized",
  "egress_pool": null,
  "egress_source": null,
  "feedback_report": null,
  "meta": {},
  "headers": {
    "Subject": "test smtp xx.xx.xx.xxx--"
  },
  "delivery_protocol": null,
  "reception_protocol": "ESMTP",
  "nodeid": "dd2b41fd-78f0-4105-8cd9-01ac7114cada"
}
```

This occurs because of the `log_oob` and `log_arf` options being enabled for `*` in the configuration.

Whenever any of the `relay_to`, `log_oob`, or `log_arf` options are set to `true`, KumoMTA cannot immediately reject connections from hosts not listed in `relay_hosts` because it needs to see whether the remote host is sending messages to one of the domains listed in the `listener_domains` helper configuration.

After a `RCPT TO` command is received, KumoMTA will check whether the destination domain is listed in the `listener_domain` config and if so, will receive the message and act accordingly. This can result is messages being received that may or may not be queued after being processed as an OOB and/or ARF message, depending on whether `relay_to` is set to `true`.
