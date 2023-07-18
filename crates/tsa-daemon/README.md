# Traffic Shaping Automation Daemon

## Overview

tsa-daemon will accept delivery logs via:

* HTTP
* AMQP (will subscribe to a log topic)

The logs will be ingested and have a set of user-defined rules applied:

* Matching:
 * The DSN from the log record will be matched via a regex
 * or: Bounce Classification string
 * Scoping based on destination domain name or site name

* Triggering criteria: additional fields can refine the match:
 * Number of events in a time window
 * or: Trigger immediately

The actions that can be taken:

* Suspend delivery to the destination domain or site name for a time period
* Generate traffic shaping configuration overrides for a domain or site name
  that will last for a specified time period
* Clear traffic shaping configuration overrides for a domain or site name

## Ingestion considerations

Log records may have been queued by the sender in some unusual circumstances.
We should take care to analyze the timestamp in the incoming log record
and use that accordingly.  For example, if the action(s) applied by a rule
would last 2 hours in duration, and the record is 3 hours old, then we should
skip applying those rules.

## State

Each rule will need to be hashed to produce a stable identifier so that we
can keep track of the number of matches in the appropriate time window, and
so that we can identify the corresponding actions and extend their effects
if another record re-triggers the rule.

When shutting down, we should record the counts for the current time window
and its bounds so that we can restore it accordingly when we are restarted.

## Publishing the actions

tsa-daemon will serve the active set of actions via HTTP in the following
forms:

* A json or toml data file (we can serve both; the toml version can include
  comments about the triggering criteria and duration) that is compatible
  with the shaping helper functionality in kumomta.  The intent is to
  adjust the shaping helper to supporting pulling from that data source
  and layering it over the other data files.
* A json or toml data file listing any suspensions.  This is primarily
  informational

