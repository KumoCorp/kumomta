# Configuring Listeners

An SMTP listener can be defined with a ```kumo.start_esmtp_listener``` function.  In the example below you can see the definition of IP address, Port, and specific relay hosts that are permitted to to use that listener.

Each listener can have its own relay list, banner, hostname and list of controls to determine domain behavior.
```console
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

  Refer to the Reference Manual for detailed options: 
  https://docs.kumomta.com/reference/kumo/start_esmtp_listener/
  