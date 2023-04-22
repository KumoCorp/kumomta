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