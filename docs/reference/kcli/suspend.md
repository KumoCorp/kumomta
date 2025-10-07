# kcli suspend


Administratively suspend messages in matching queues


**Usage:** `kcli suspend [OPTIONS] --reason <REASON> <--domain <DOMAIN>|--campaign <CAMPAIGN>|--tenant <TENANT>|--everything|--queue <QUEUE>>`

## Options


* `--domain <DOMAIN>` — The domain name to match. If omitted, any domains will match!

* `--campaign <CAMPAIGN>` — The campaign name to match. If omitted, any campaigns will match!

* `--tenant <TENANT>` — The tenant name to match. If omitted, any tenant will match!

* `--reason <REASON>` — The reason to log in the delivery logs

* `--everything` — Suspend all queues

* `--queue <QUEUE>` — Suspend specific scheduled queue names using their exact queue name(s). Can be specified multiple times

* `--duration <DURATION>` — The duration over which matching messages will continue to suspend. The default is '5m'



