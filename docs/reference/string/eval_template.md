---
tags:
  - templates
---

# kumo.string.eval_template

```lua
kumo.string.eval_template(NAME, SOURCE, CONTEXT)
```

{{since('2025.03.19-1d3f1f67')}}

Evaluates a minijinja template.

The parameters are:

* `NAME` - the nominal "file name" for the template. No actual file I/O is
  performed; this parameter tells the template engine what the filename would
  have been if it were loaded from disk. The filename is used to decide what
  strategy should be used for automatic output escaping.  For example, if the
  name ends with `.json` or `.html` then appropriate json or html entity
  escaping will automatically be applied to the output.
* `SOURCE` - the template source code. This must follow the [template syntax](../template/index.md).
* `CONTEXT` - an object that defines the variables that are available to the template engine.

```lua
-- This will print `"John"`
print(
  kumo.string.eval_template('example.json', [[{{name}}]], { name = 'John' })
)
```


```lua
local kumo = require 'kumo'

local log_record = kumo.serde.json_parse [=[
{
    "type": "Delivery",
    "id": "1d98076abbbc11ed940250ebf67f93bd",
    "sender": "user@sender.example.com",
    "recipient": "user@recipient.example.com",
    "queue": "campaign:tenant@domain",
    "site": "source2->(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com.",
    "size": 1047,
    "response": {
        "code": 250,
        "enhanced_code": {
            "class": 2,
            "subject": 0,
            "detail": 0
        },
        "content": "OK ids=8a5475ccbbc611eda12250ebf67f93bd",
        "command": "."
    },
    "peer_address": {
        "name": "gmail-smtp-in.l.google.com.",
        "addr": "142.251.2.27"
    },
    "timestamp": 1678069691,
    "created": 1678069691,
    "num_attempts": 0,
    "bounce_classification": "Uncategorized",
    "egress_pool": "pool0",
    "egress_source": "source2",
    "source_address": {
        "address": "10.0.0.1:53210",
        "protocol": "socks5",
        "server": "192.168.1.1:5000"
    },
    "feedback_report": null,
    "meta": {},
    "headers": {},
    "delivery_protocol": "ESMTP",
    "reception_protocol": "ESMTP",
    "bogus": null,
    "nodeid": "557f3ad4-2c8c-11ee-976e-782d7e12e173",
    "tls_cipher": "TLS_AES_256_GCM_SHA384",
    "tls_protocol_version": "TLSv1.3",
    "tls_peer_subject_name": ["C=US","ST=CA","L=SanFrancisco","O=Fort-Funston",
                              "OU=MyOrganizationalUnit","CN=do.havedane.net",
                              "name=EasyRSA","emailAddress=me@myhost.mydomain"],
    "session_id": "9bcd689e-23d9-41b7-a015-63a1382f8b57"
}
]=]

print(kumo.string.eval_template(
  'log.json',
  [[{
{%- for key, value in log_record | items | reject("none") -%}
{{key}}: {{value }}
{%- if not loop.last %},{% endif %}
{%- endfor -%}
}
]],
  { log_record = log_record }
))
```
