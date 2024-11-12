# Configuration Lifecycle

Configuration in kumomta is expressed through the policy script that `kumod` is
instructed to start with. The default value for the policy script is
`/opt/kumomta/etc/policy/init.lua`.

When the service is started the policy script is evaluated and the policy will
then register a number of event handler functions whose purpose it is to
provide configuration information to the system.

This enables you to have a very dynamic setup that *could* be fed from an
online data source, if desired, although you should carefully consider
how you want your deployment to behave if such a data source is transiently
unavailable before you go down that path.

The main configuration events are:

* [init](events/init.md) - called only once, when `kumod` is initializing
  the service. The goal is to define listeners for SMTP, HTTP, logging and
  spool, and perform other one-time initialization.

* [get_listener_domain](events/get_listener_domain.md) - called during an
  SMTP transaction to resolve relaying information about sending and receiving
  domains.

* [get_queue_config](events/get_queue_config.md) - called in response to
  enqueuing a message to a scheduled queue which has not yet been instantiated
  in the system, and when the system decides that that information needs to be
  refreshed--see more on that below. This event uses
  [make_queue_config](kumo/make_queue_config/index.md) to create a queue
  configuration object to communicate the settings to the system.

* [get_egress_pool](events/get_egress_pool.md) - typically called right
  after `get_queue_config` to resolve the egress pool that was specified
  for that queue, if we don't already have that information cached.

* [get_egress_source](events/get_egress_source.md) - called to resolve
  an individual source as part of setting up a scheduled queue if we
  don't have that information cached.

Configuration data is kept primarily only in the associated objects (such as
queues) while those objects are live and required by the system, aging out as
the corresponding queue is emptied and idles out of the system.  This means
that we do not need to keep a massive static configuration in memory for
the lifetime of the service.

!!! note
    KumoMTA provides a number of higher level *helper* modules that
    provide implementations of these underlying event handlers, so
    in the most common usage scenarios you don't need to get into
    the details of most of these event handlers.

## Configuration Caching and Refreshing

KumoMTA provides two different configuration refresh strategies:

* `Ttl` - the object will be considered stale after its refresh interval
  has expired, which will lead to the corresponding event handler being
  triggered to refresh its state.  In a system with many queues, this
  can lead to periodic busy cycles to ensure that the configuration is
  current.

* `Epoch` - the object will remain valid until the *configuration epoch* (see
  below) changes.  This results in fewer speculative periodic calls to
  the associated event handlers, but is harder to use in concert when
  pulling configuration from remote data sources.

## Config Epoch

{{since('2024.11.08-d383b033')}}

KumoMTA monitors your local configuration files and computes a hash
of their contents.  When that hash changes, which signals an observable
change in something in those configuration files, it marks the start
of a new *configuration epoch*, and bumps the current epoch number
up by one.

This change of epoch can be used as a signal to update/refresh
information that is derived from your configuration files.

### Configuration Monitoring

Configuration monitoring uses a fairly simplistic filesystem polling mechanism
that, every 10 seconds, evaluates a set of *glob* expressions to determine the
list of files that should be considered to factor into the overall
configuration state.

This polling system intentionally does **not** use an OS-level file monitoring
facility because it is not possible to efficiently do that with a glob
expression, and also because those facilities are not guaranteed to work in
every type of container solution.

The default set of globs that are monitored is:

* `/opt/kumomta/etc/**/*.{lua,json,toml,yaml}`

which matches the most likely sources of configuration code and data.

You can use [kumo.set_config_monitor_globs](kumo/set_config_monitor_globs.md)
to change the set of globs if your deployment requires it.

### Explicitly Bumping the Config Epoch

The Epoch system will automatically notice and react to changes to the local
files that match its glob expression, but if you are dependent upon
non-filesystem data sources, such as TSA daemon, or other remote databases, it
is necessary to provide a way to signal to KumoMTA that it should re-assess the
configuration.

There are two ways that this is achieved:

* [kumo.bump_config_epoch](kumo/bump_config_epoch.md) is a lua function that
  can be called in code to force the epoch to increment and to notify other
  modules about the change.  This mechanism is used by our shaping helper in
  response to a websocket push from the TSA daemon to cause its configuration
  updates to be picked up.

* [/api/admin/bump-configuration](rapidoc.md#post-/api/admin/bump-config-epoch)
  is an HTTP endpoint that can be used to externally bump the configuration
  epoch. This can be useful as part of a deployment process or configuration
  update happening elsewhere in your infrastructure.

