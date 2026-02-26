---
tags:
 - logging
---

# kumo.aaa.configure_acct_log

{{since('dev')}}

```lua
kumo.on('init', function()
  kumo.aaa.configure_acct_log(PARAMS)
end)
```

This function configures and enables the *accounting* logs for both
authentication and authorization.

This function is intended to be called during the `init` or `pre_init` event callback.
Calling it multiple times is an error.

`PARAMS` is an object style lua table that can have fields as described in the
[Log Parameters](#log-parameters) section below.

The minimal configuration looks like:

```lua
kumo.on('init', function()
  kumo.aaa.configure_acct_log {
    log_dir = '/var/log/kumo-logs/acct',
    -- If you find that the logs are big and busy, you might consider
    -- disabling logging successful authentication and authorization
    -- events, leaving the logs to focus on failures
    -- log_authn_ok = false,
    -- log_authz_allow = false,
  }
end)
```

Logs are written as zstd-compressed log file segments under the specified
`log_dir` directory. Each line of the file is a JSON object holding information
about an authentication or authorization event.

## Log Records

There are two types of records that could be logged to the accounting log:

### Authentication Events

```json
{
  // Indicates that this is an Authentication record
  "type": "Authentication",
  // When the event occurred
  "timestamp":"2025-12-18T06:52:47.798373949Z",
  // Who were they trying to authenticate as?
  "attempted_identity": {
    // the username, if known (in some contexts, we may not
    // have a human readable identity string, and this may be
    // an empty string)
    "identity": "daniel",
    // How were they trying to authenticate?
    "context": "HttpBasicAuth"
  },
  // Was authentication successful?
  "success": false,
  // The auth info from the session, which will include the
  // attempted_identity if the authentication was successful
  "auth_info": {
    // The IP address of the system making the request
    "peer_address": "127.0.0.1",
    // The set of identities which are active for the session
    "identities": [],
    // any groups that are active for the session
    "groups": [
      "kumomta:http-listener-trusted-ip"
    ]
  }
}
```

### Authorization Events

```json
{
  // Indicates that this is an Authorization record
  "type": "Authorization",
  // When the event occurred
  "timestamp":"2025-12-18T06:52:47.798373949Z",
  // Which resource was being accessed?
  "target_resource": "http_listener/0.0.0.0:8000/api/admin/suspend/v1",
  // What privilege was being requested of that resource?
  "privilege": "GET",
  // Was access `Allow`d or `Deny`d?
  "access": "Allow",
  // If an ACL rule was responsible for deciding the access level,
  // which resource did that rule belong to?  This aids in tracing
  // and debugging ACL rules.  This may be missing or null if
  // no rule matched.
  "matching_resource": "http_listener/*/api/admin",
  // If a rule matched, this is a copy of that matching rule, to aid
  // in tracing and debugging ACL rules.
  "rule": {
    "criteria": {
      "Identity": {
        "Group": "kumomta:http-listener-trusted-ip"
      }
    },
    "privilege": "GET",
    "access": "Allow"
  },
  // A copy of the auth info from the session
  "auth_info": {
    "peer_address": "127.0.0.1",
    "identities": [],
    "groups": [
      "kumomta:http-listener-trusted-ip"
    ]
  },
  // A list of the resources that were considered, in the order that
  // they were considered, before we reached the matching_resource.
  // This aids in tracing and debugging ACL rules.
  "considered_resources": [
    "http_listener/0.0.0.0:8000/api/admin/suspend/v1",
    "http_listener/0.0.0.0:8000/api/admin/suspend",
    "http_listener/0.0.0.0:8000/api/admin",
    "http_listener/0.0.0.0:8000/api",
    "http_listener/0.0.0.0:8000",
    "http_listener/*/api/admin/suspend/v1",
    "http_listener/*/api/admin/suspend"
  ]
}
```


## Log Parameters

### log_authz_allow

If set to `true` (the default), then successful authorization events will be logged.

### log_authz_deny

If set to `true` (the default), then failed authorization events will be logged.

### log_authn_ok

If set to `true` (the default), then successful authentication events will be logged.

### log_authn_fail

If set to `true` (the default), then failed authentication events will be logged.

### back_pressure

Maximum number of outstanding items to be logged before
the submission will block; helps to avoid runaway issues
spiralling out of control.

```lua
kumo.aaa.configure_acct_log {
  -- ..
  back_pressure = 128000,
}
```

### compression_level

Specifies the level of *zstd* compression that should be used.  Compression
cannot be disabled.

Specifying `0` uses the zstd default compression level, which is `3` at the
time of writing.

Possible values are `1` (cheapest, lightest) through to `21`.

```lua
kumo.aaa.configure_acct_log {
  -- ..
  compression_level = 3,
}
```

### log_dir

Specifies the directory into which log file segments will be written.
This is a required key; there is no default value.

```lua
kumo.aaa.configure_acct_log {
  -- ..
  log_dir = '/var/log/kumo-logs/acct',
}
```

### max_file_size

Specify how many uncompressed bytes to allow per file segment. When this number
is exceeded, the current segment is finished and a new segment is created.

Segments are created using the current time in the form `YYYYMMDD-HHMMSS` so that
it is easy to sort the segments in chronological order.

The default value is ~1GB of uncompressed data, which compresses down to around
50MB of data per segment with the default compression settings.

```lua
kumo.aaa.configure_acct_log {
  -- ..
  max_file_size = 1000000000,
}
```

### max_segment_duration

Specify the maximum time period for a file segment.  The default is unlimited.

If you set this to `"1min"`, you indicate that any given file should cover a
time period of 1 minute in duration; when that time period elapses, the current
file segment, if any, will be flushed and closed and any subsequent events will
cause a new file segment to be created.

```lua
kumo.aaa.configure_acct_log {
  -- ..
  max_segment_duration = '5 minutes',
}
```

### min_free_inodes

Specifies the desired minimum amount of free inodes for the log storage
in this location.  Can be specified using either a string like `"10%"` to
indicate the percentage of available inodes, or a number to indicate the
number of available inodes.

If the available inodes are below the specified amount then kumomta will
reject incoming SMTP and HTTP injection requests and the
[check-liveness](../http/kumod/api_check_liveness_v1_get.md) endpoint will indicate
that new messages cannot be received.

The default value for this option is `"10%"`.


### min_free_space

Specifies the desired minimum amount of free disk space for the log storage
in this location.  Can be specified using either a string like `"10%"` to
indicate the percentage of available space, or a number to indicate the
number of available bytes.

If the available storage is below the specified amount then kumomta will
reject incoming SMTP and HTTP injection requests and the
[check-liveness](../http/kumod/api_check_liveness_v1_get.md) endpoint will indicate
that new messages cannot be received.

The default value for this option is `"10%"`.

