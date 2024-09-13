# kcli provider-summary


Prints a summary of the aggregate state of the queues from the perspective of the provider or destination site.

Note that this output format is subject to change and is not suitable for a machine to parse. It is expressly unstable and you must not depend upon it in automation.

The data behind this output is pulled from the metrics endpoint, which is machine readable.

The default output mode is to show the total volume of traffic grouped by the provider, or, if not applicable provider matching rules were defined on the server, the site name that is derived from the MX records for a domain.

The data is shown ordered by descending volume, where volume is the sum of the delivered, failed, transiently failed and queued message counts.

The --by-pool flag will further sub-divide the display by the egress pool.

The column labels have the following meanings:

PROVIDER - either the provider (if explicitly set through the config on the server), or the site name for the underlying domain.

POOL     - (when --by-pool is used) the name of the egress pool

D        - the total number of delivered messages

T        - the total number of transiently failed messages

F        - the total number of failed/bounced messages

Q        - the total number of ready and scheduled messages in queue

C        - the current number of open connections

DOMAINS  - (when --show-domains is used) a list of domains that correspond to rows that do not have an explicitly configured provider.


**Usage:** `kcli provider-summary [OPTIONS]`

## Options


* `--by-pool` — Include a POOL column in the output, and break down the volume on a per-pool basis

* `--show-domains` — For rows that were not matched on the server by provider rules we will normally show a site-name in the place of the PROVIDER column.

     When --show-domains is enabled, an additional DOMAINS column will be added to the output to hold a list of domains which correspond to that site-name.

     This option is only capable of filling in the list of domains if any of those domains have delayed messages residing in their respective scheduled queues at the time that this command was invoked.

* `--limit <LIMIT>` — Limit results to LIMIT results



