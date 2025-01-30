# reconnect_strategy

{{since('2025.01.29-833f82a8')}}

Controls the behavior of the SMTP dispatcher when it encounters an error during
message delivery.  It does **not** affect connection-time errors (such as connection
failure, or protocol or transport errors around banner, EHLO, STARTTLS, AUTH),
and specifically only targets errors that might arise as part of delivering
a message:

* A 421 protocol response where the peer closes the connection.
* A timeout writing a request
* A timeout reading a response
* Some other IO error on the transport (eg: connection reset)

You can set the `reconnect_strategy` to one of the following values to select
the desired behavior for session re-use for subsequent messages:

* `"TerminateSession"` - close the current session, allowing the queue
  maintainer to decide about opening a new connection based on your shaping
  configuration.  If a new session is established, it will start with a fresh
  connection plan.
* `"ReconnectSameHost"` - close the current connection, but adjust the session
  state so that it will try connecting to the same host again for future sends.
* `"ConnectNextHost"` - close the current connection and proceed to the next
  host in the connection plan. This is the default behavior.

The connection plan is constructed when a session is initiated; it is drawn
from the preference-ordered list of MX hosts, but randomizes the set of hosts
at each preference level.  This is then flattened into a list of hosts that
will be attempted one after the other to establish a connection.

`"ConnectNextHost"` will maximize the chances of delivering mail in the face of
various transient issues with the destination site.

Some sites have very opininated anti-abuse policies and consider any attempt to
connect to second tier (non-preferred) MX hosts as signs of bad behavior and
this may impact your effective deliverability.  For those sites you may want to
consider deploying with `reconnect_strategy="TerminateSession"`.

