# check

{{since('dev')}}

```lua
local mail_auth = require 'policy-extras.mail_auth'
local check_result = mail_auth.check(MSG, OPT_CONFIG)
```

The `mail_auth.check` function performs a bundle of standard authentication
checks, collecting together the authentication results.

There are no policy decisions encoded within the check; the various checks are
carried out with their status reported in the returned object.  It up to you to
interpret the results and apply your policy based on those results.

The parameters are:

 * `MSG` - a required [Message](../message/index.md) object to be checked
 * `OPT_CONFIG` - an optional configuration object described below

The optional configuration object is an object style lua table with the
following fields; all fields are optional:

 * `dkim` - a boolean, which defaults to `true`, indicating whether
   [msg:dkim_verify](../message/dkim_verify.md) should be called and the
   results collected.
 * `spf` - a boolean, which defaults to `true`, indicating whether
   [kumo.spf.check_msg](../kumo.spf/check_msg.md) should be called and the
   result collected.
 * `iprev` - a boolean, which defaults to `true`, indicating whether
   [policy-extras.mail_auth.iprev_msg](iprev_msg.md) should be called and
   the result collected.
 * `smtp_auth` - a boolean, which defaults to `true`, indicating whether
   the [SMTP
   authentication](https://datatracker.ietf.org/doc/html/rfc8601#autoid-24)
   status should be collected
 * `dmarc` - a boolean, which defaults to `true`, indicating whether DMARC
   result should be collected.
 * `arc` - a boolean, which defaults to `true`, indicating whether
   [msg:arc_verify](../message/arc_verify.md) should be called and the result
   collected.
 * `add_authentication_results` - a boolean, which defaults to `true`, indicating
   whether the aggregated authentication results performed by `check` should be
   added to the message as an
   [Authentication-Results](https://datatracker.ietf.org/doc/html/rfc8601#autoid-1)
   header via [msg:add_authentication_results](../message/add_authentication_results.md).
 * `server_id` - a string which specifies the `server_id` parameter that should be
   passed to [msg:add_authentication_results](../message/add_authentication_results.md)
   when `add_authentication_results` is enabled.  If you do not specify `server_id`
   then the `hostname` metadata value will be extracted from the `MSG`.
 * `resolver` - a string corresponding to the name of a resolver defined via
   [kumo.dns.define_resolver](../kumo.dns/define_resolver.md) for more advanced
   use cases.  The default behavior is to use the overall KumoMTA DNS resolver
   configuration.

The return value of `mail_auth.check` is an object with the following fields:

 * `dkim` - an array style table holding the list of DKIM
   [authenticationresult](../authenticationresult.md)s, one for each signature,
   or a single one indicating that there was no signature.  If `OPT_CONFIG.dkim
   == false` then this field will be absent.
 * `spf` - The [authenticationresult](../authenticationresult.md) produced by
   the spf check.  If `OPT_CONFIG.spf == false` then this field will be absent.
 * `iprev` - The [authenticationresult](../authenticationresult.md) produced by
   the iprev check.  If `OPT_CONFIG.iprev == false` then this field will be absent.
 * `smtp_auth` - The [authenticationresult](../authenticationresult.md) produced by
   the SMTP authentication check.  If `OPT_CONFIG.smtp_auth == false` then this
   field will be absent.
 * `dmarc` - The [authenticationresult](../authenticationresult.md) produced by
   the DMARC check.  If `OPT_CONFIG.dmarc == false` then this field will be absent.
 * `arc` - The [authenticationresult](../authenticationresult.md) produced by
   the ARC check.  If `OPT_CONFIG.arc == false` then this field will be absent.
 * `auth_results` - An array style table holding the list of all
   [authenticationresult](../authenticationresult.md)s produced by the checks
   that we carried out.  This is useful to pass onwards to
   [msg:arc_seal](../message/arc_seal.md).

## Example

```lua
local mail_auth = require 'policy-extras.mail_auth'

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Perform checks, an annotate the message with an Authentication-Results header
  local check_result = mail_auth.check(msg)

  -- You could pass on check_result.auth_results to msg:arc_seal here if
  -- you are signing and sealing messages with ARC
end)
```
