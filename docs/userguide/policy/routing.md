# Configuring Message Routing

If you need to "smarthost" or route messages through another server, you have
several options.

## Changing the routing domain at reception time (per-message)

{{since('2023.08.22-4d895015', indent=True)}}
    At reception time, you can specify an alternate *routing domain* for a
    message.  Messages with the same *destination domain* (based on their
    recipients) and *routing domain* will be queued into a separate
    *scheduled queue* from their normal destination domain.

    This is conceptually similar to the `queue` rewriting approach mentioned
    below, but the original destination domain, tenant and campaign information
    is preserved, and multiple separate scheduled queues are created to manage
    them.

    The example below will unconditionally smarthost all incoming messages to
    `my.smarthost.com`.  Mail originally destined for `example.com` will be queued
    into a scheduled queue named `example.com!my.smarthost.com` so that it can
    be managed independently of other domains.

    When delivering these messages, the regular MX resolution process will be
    performed but using `my.smarthost.com` instead of the recipient domain.

    This must be carried out in your
    [smtp_server_message_received](../../reference/events/smtp_server_message_received.md)
    or [http_message_generated](../../reference/events/http_message_generated.md)
    event handler.

    ```lua
    kumo.on('smtp_server_message_received', function(msg)
        msg:set_meta('routing_domain', 'my.smarthost.com')
    end)
    ```

## Explicitly overriding the MX resolution for a scheduled queue (domain-based)

{{since('2023.08.22-4d895015', indent=True)}}
    If you are re-routing a domain to internal infrastructure that doesn't have MX
    records, then this technique may be suitable.  It works by overriding the
    MX resolution that would normally be used for a scheduled queue.

    The override is performed by setting the configuration for the scheduled queue
    using the [get_queue_config](../../reference/events/get_queue_config.md) event:

    ```lua
    kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
        if domain == 'domain.to.be.smart.hosted' then
            -- Relay via some other internal infrastructure.
            -- Enclose IP (or IPv6) addresses in `[]`.
            -- Otherwise the name will be resolved for A and AAAA records
            return kumo.make_queue_config {
                protocol = {
                    smtp = {
                        mx_list = { 'smart.host.local', { name = 'mx.example.com', addr = '10.0.0.1' }}
                    },
                },
            }
        end
        -- Otherwise, just use the defaults
        return kumo.make_queue_config {}
    end)
    ```

    This approach will resolve `A`/`AAAA` records but not `MX` records for
    the list of hosts in `mx_list`.  `mx_list` is used as the ordered list
    of hosts to which the message should be delivered.  It is used in
    place of the normal MX resolution that would have been carried out
    for the domain.

    With this approach, the original scheduled queue name remains as it
    was.

## Rewriting the queue at reception time (per-message)

!!! note
    Using the `routing_domain` approach mentioned above is generally
    preferred to this approach, as it preserves tenant and campaign
    information with no additional work required on your part.

At reception time, you can override the default scheduled queue that a message
will be placed into.  The original recipient domain, campaign and tenant
information are effectively ignored when using this technique.

The example below will unconditionally assign all incoming messages to the
scheduled queue for `my.smarthost.com`.

When delivering these messages, the regular MX resolution process will be
performed but using `my.smarthost.com` instead of the recipient domain.

This must be carried out in your
[smtp_server_message_received](../../reference/events/smtp_server_message_received.md)
or [http_message_generated](../../reference/events/http_message_generated.md)
event handler.

```lua
kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('queue', 'my.smarthost.com')
end)
```

## A note on IPv4 and IPv6 literal Addresses

When rewriting the routing domain or queue, it is possible to specify literal
addresses instead of DNS names, but those must still be compliant with the SMTP
specification which requires that literal address domains be enclosed in square
brackets.

For example:

  * `[10.0.0.1]` is a valid IPv4 domain literal
  * `[IPv6:::1]` is a valid IPv6 domain literal representing the `::1` address.
  * `[::1]` is an **_invalid, non-conforming_** IPv6 domain literal, because it is
    missing the `IPv6:` address tag prefix, but is accepted by KumoMTA and treated
    as an IPv6 address. In the context of smart-hosting, this is no problem, but
    in general we do not recommend using this non-conforming syntax in the envelope
    or body of your messages as it may not be supported by downstream MTAs.

```lua
kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('queue', '[20.83.209.56]')
end)
```

