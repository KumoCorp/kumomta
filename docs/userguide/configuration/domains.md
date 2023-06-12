# Configuring Inbound and Relay Domains

When configuring an SMTP listener, it is common to simply define a list of `relay_hosts` that are permitted to connect and relay messages through the server. Any host that connects and does not match the list of relay hosts will be rejected immediately.

While a simple list of relay_hosts is appropriate for outbound relay, even organizations focused on email sending will also need to receive inbound messages in the form of out-of-bound bounce messages, AKA Message Disposition Notification (MDN) messages, as well as Feedback Loop (FBL) abuse reports in Abuse Reporting Format (ARF).

Additionally, some mixed-sending environments may need to add additional security by limiting which domains a given injecting host is permitted to send from. This helps prevent a malicious system from impersonating a peer by injecting messages with the peer's domain, something possible when only relay_hosts are configured.

To address these use cases, listeners can be configured with domain-level configuration:

!!!note
    It is a best practice to use separate listeners for hosts inside and outside of your network. Typically the `relay_to`, `log_oop`, and `log_arf` options would be configured for listeners on the external network, where the `relay_from` option could appear on both internal and external listeners.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',

  -- override the default set of relay hosts
  relay_hosts = { '127.0.0.1', '192.168.1.0/24' },

  -- Configure the domains that are allowed for outbound & inbound relay,
  -- Out-Of-Band bounces, and Feedback Loop Reports.
  -- See https://docs.kumomta.com/userguide/configuration/domains/
  domains = {
    ['examplecorp.com'] = {
      -- allow relaying mail from any source, so long as it is
      -- addressed to examplecorp.com, for inbound mail.
      relay_to = true,
    },
    ['send.examplecorp.com'] = {
      -- relay to anywhere, so long as the sender domain is
      -- send.examplecorp.com and the connected peer matches one of the
      -- listed CIDR blocks, helps prevent abuse by less trusted peers.
      relay_from = { '10.0.0.0/24' },
    },
    ['bounce.examplecorp.com'] = {
      -- accept and log OOB bounce reports sent to bounce.examplecorp.com
      log_oob = true,
    },
    ['fbl.examplecorp.com'] = {
      -- accept and log ARF feedback reports sent to fbl.examplecorp.com
      log_arf = true,
    },
  },
}
```

Additional information on the log_oob and log_arf options can be found in the [Configuring Bounce Classification](./bounce.md) and the [Configuring Feedback Loop Processing](./fbl.md) chapters respectively.

## Configuring Domains for Relaying, Bounces, and Feedback Loops

By default, if a host connects to the listener and is not listed in the `relay_hosts` directive, that host will not be permitted to inject messages. While that is acceptable for most outbound relaying, it is necessary in most use cases to also configure certain exceptions on a destination-domain basis for things such as inbound mail relay, out-of-band bounce processing, and feedback loop processing.

When a host not on the relay_hosts list connects to a listener and issues a `RCPT TO` command, the `get_listener_domain` hook is fired, allowing for policy to be applied based on the destination domain of the message.

To apply policy based on the domain, the `make_listener_domain` function is used:

```lua
kumo.on('get_listener_domain', function(domain)
  if domain == 'example.com' then
    return kumo.make_listener_domain {
      relay_to = true,
    }
  end
end)
```

In the preceding example, the domain `example.com` is permitted for inbound relay, resulting in messages destined for the domain being accepted and queued.

For more information, see the [make_listener_domain](../../reference/kumo/make_listener_domain.md) page of the Reference manual for more information.

## Using the listener_domains.lua Policy Helper

For most basic use cases, it will be simpler to use the `listener_domains.lua` policy helper script to manage the listener domain configuration in a simple TOML file.

To use the helper, add the following to the top level of your server policy script:

```lua
local listener_domains = require 'policy-extras.listener_domains'

kumo.on(
  'get_listener_domain',
  listener_domains:setup { '/opt/kumomta/etc/listener_domains.toml' }
)
```

Then create a text file at `/opt/kumomta/etc/listener_domains.toml` with the following format:

```toml
["example.com"]
# allow relaying mail from anyone, so long as it is
# addressed to example.com
relay_to = true

["bounce.example.com"]
# accept and log OOB bounce reports sent to bounce.example.com
log_oob = true

["fbl.example.com"]
# accept and log ARF feedback reports sent to fbl.example.com
log_arf = true

["send.example.com"]
# relay to anywhere, so long as the sender domain is send.example.com
# and the connected peer matches one of the listed CIDR blocks
relay_from = { '10.0.0.0/24' }

# wildcards are permitted. This will match
# <anything>.example.com that doesn't have
# another non-wildcard entry explicitly
# listed in this set of domains.
# Note that "example.com" won't match
# "*.example.com".
["*.example.com"]
# You can specify multiple options if you wish
log_oob = true
log_arf = true
relay_to = true

# and you can explicitly set options to false to
# essentially exclude an entry from a wildcard
["www.example.com"]
relay_to = false
log_arf = false
log_oob = false

# Define a per-listener configuration
[listener."127.0.0.1:25"."*.example.com"]
log_oob = false
```