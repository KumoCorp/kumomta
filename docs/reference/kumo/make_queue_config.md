# `kumo.make_queue_config { PARAMS }`

Constructs a configuration object that specifies how a *queue* will behave.

This function should be called from the
[get_queue_config](../events/get_queue_config.md) event handler to provide the
configuration for the requested queue.

The following keys are possible:

## egress_pool

The name of the egress pool which should be used as the source of
this traffic.

If you do not specify an egress pool, a default pool named `unspecified`
will be used. That pool contains a single source named `unspecified` that
has no specific source settings: it will just make a connection using
whichever IP the kernel chooses.

See [kumo.define_egress_pool()](define_egress_pool.md).

## max_age

Limits how long a message can remain in the queue.
The default value is `"7 days"`.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign)
  return kumo.make_queue_config {
    -- Age out messages after being in the queue for 20 minutes
    max_age = '20 minutes',
  }
end)
```

## max_retry_interval

Messages are retried using an exponential backoff as described under
*retry_interval* below. *max_retry_interval* sets an upper bound on the amount
of time between delivery attempts.

The default is that there is no upper limit.

The value is expressed in seconds.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign)
  return kumo.make_queue_config {
    -- Retry at most every hour
    max_retry_interval = '1 hour',
  }
end)
```

## protocol

Configure the delivery protocol. The default is to use SMTP to the
domain associated with the queue, but you can also configure delivering
to a local [maildir](http://www.courier-mta.org/maildir.html):

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign)
  if domain == 'maildir.example.com' then
    -- Store this domain into a maildir, rather than attempting
    -- to deliver via SMTP
    return kumo.make_queue_config {
      protocol = {
        maildir_path = '/var/tmp/kumo-maildir',
      },
    }
  end
  -- Otherwise, just use the defaults
  return kumo.make_queue_config {}
end)
```

!!! note
    Maildir support is present primarily for functional validation
    rather than being present as a first class delivery mechanism.

Failures to write to the maildir will cause the message to be delayed and
retried approximately 1 minute later.  The normal message retry schedule does
not apply.

## retry_interval

Messages are retried using an exponential backoff.  *retry_interval* sets the
base interval; if a message cannot be immediately delivered and encounters a
transient failure, then a (jittered) delay of *retry_interval* seconds will be
applied before trying again. If it transiently fails a second time,
*retry_interval* will be doubled and so on, doubling on each attempt.

The default is `"20 minutes"`.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign)
  return kumo.make_queue_config {
    retry_interval = '20 minutes',
  }
end)
```
