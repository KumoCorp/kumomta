# Configuring Inbound and Relay Domains

When listening via SMTP, it is common to simply define a list of `relay_hosts`
that are permitted to connect and relay messages through the server. Any host
that connects and does not match the list of relay hosts will be allowed to
connect to the server, but will not be permitted to relay mail through
the server.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',

  -- override the default set of relay hosts
  relay_hosts = { '127.0.0.1', '192.168.1.0/24' },
}
```

## Using the listener_domains.lua Policy Helper

For most basic use cases, it will be simpler to use the `listener_domains.lua`
policy helper script to manage the listener domain configuration in a simple
TOML file.

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
["*"]
# You can specify * as a default, overridden by any more explicitly defined domains.
# Since all options are false by default, this would only be needed to default
# An option to true for all domains.
relay_to = false
log_oob = true
log_arf = true

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
relay_from = [ '10.0.0.0/24' ]

["auth-send.example.com"]
# relay to anywhere, so long as the sender domain is auth-send.example.com
# and the connected peer has authenticated as any of the authorization identities
# listed below using SMTP AUTH
relay_from_authz = [ 'username1', 'username2' ]

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

## Configuring Domains for Relaying, Bounces, and Feedback Loops

By default, if a host connects to the listener and is not listed in the
`relay_hosts` directive, that host will not be permitted to inject messages.
While that is acceptable for most outbound relaying, it is necessary in most
use cases to also configure certain exceptions on a destination-domain basis
for things such as inbound mail relay, out-of-band bounce processing, and
feedback loop processing.

When a host not on the relay_hosts list connects to a listener and issues a
`RCPT TO` command, the
[get_listener_domain](../../reference/events/get_listener_domain.md) hook is
fired, allowing for policy to be applied based on the destination domain of the
message.

To apply policy based on the domain, the `make_listener_domain` function is used:

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if domain == 'example.com' then
    return kumo.make_listener_domain {
      relay_to = true,
      log_oob = true,
      log_fbl = true,
    }
  end
end)
```

In the preceding example, the domain `example.com` is permitted for inbound
relay, resulting in messages destined for the domain being accepted and queued,
while messages will also be checked for whether they are OOB or FBL messages,
which are processed, logged, and discarded.

Additional information on the `log_oob` and `log_arf` options can be found in the
[Configuring Bounce Classification](bounce.md) and the [Configuring Feedback
Loop Processing](fbl.md) chapters respectively.

For more information, see the
[make_listener_domain](../../reference/kumo/make_listener_domain.md) page of
the Reference manual.
