# Hornetsecurity Spam and Malware Protection integration

## Introduction

This integration makes **Hornetsecurity Email Protection** and the
**Hornetsecurity Filter Engine** available for KumoMTA to scan messages in
real-time.

Only the "scan" function is implemented for in-line use with KumoMTA.

## Install

If you have not already done so, contact
[Hornetsecurity](https://www.hornetsecure.com/) for documentation, binary and
license.  Configure Hornetsecurity Email Protection as per their documentation.

EG:

```console
$ sudo dpkg -i hornetsecurity-emailprotection_5.0.0_amd64.deb
```

Then edit settings in
`/opt/hornetsecurity/emailprotection/etc/emailprotection.toml` including the
filter LICENSE and PROFILE

Add `local hornet = require 'policy-extras.hornet'` to the top level of init.lua with the other requires.

Call the hornet service from inside init.lua with `hornet:<function>(params)`.  See usage details below

## API License Key

You need a license key to use the Hornetsecurity Email Protection. This key
will be provided to you by Hornetsecurity.

This key must be appended to the Email Protection configuration file.

## Usage

### hornet:connect

To connect to a **Hornetsecurity Email Protection** service use `hornethost = hornet:connect(host, params)` in the top level of init.lua.

```
Inputs:
  host: Hornet Service hostname or IP address (string)
  params: Array of options including
       PORT (integer, default = 8080),
       TIMEOUT (integer, Time in seconds. Default =  10),
       USE_TLS (boolean, To use TLS or not),

Returns: Table of Connection Parameters
```

### ping, version, enginestatus, update

These API functions are not necessary and are not directly supported within KumoMTA.  All of these are accessible with simple cURL calls in the command shell.  Please refer to the Hornetsecurity documentation.


### hornet:scan

To scan a message, use `result = hornet:scan(hornethost,extraparams,msg)` in any event that can access the full message content. EG: `smtp_server_message_received`

Note that `hornet:connect` must be called prior to `hornet:scan`.
```
Inputs:
  hornethost: Hornet service host object
  extraparams: Table of:
     addheaders = true/false   -- Add result headers directly to the message?
     mode = string            -- make this smtpout (default) to indicate outbound scanning, smtpin for inbound
     x_sanitize = true/false  -- make this true to have the engine remove dangerous attachments
  msg: The KumoMTA message variable

Returns: array containing the result of the scan
IE: "200 OK: {"state":1,"score":250,"verdict":"spam:low","spamcause":"gggr...omh","elapsed":"14ms"}"
```

If the extra parameter `addheaders` = `true`, then the scan result headers will be added directly to the message before delivery.

```
IE:
X-Hornet-spamcause: gggr...omh
X-Hornet-verdict: malware
X-Hornet-elapsed: 6ms
X-Hornet-state: 2
X-Hornet-score: 9999
```

Note that messages will not be quarantined or dropped automatically regardless of the `verdict` or `score`.  We recommend you code actions based on the Hornetsecurity best practices documentation.

```lua
if result.verdict == 'malware' then
  kumo.reject(552, 'Bounced for detected malware')
end
```


## Example code

```lua
local hornet = require 'policy-extras.hornet'

kumo.on('smtp_server_message_received', function(msg)
  -- Connect to Hornetsecurity service on the local node
  print 'Checking the Hornet Server'
  local hornethost =
    hornet:connect('172.31.17.26', { port = '8080', use_tls = 'false' })

  if not hornethost then
    print 'No connection to Hornetsecurity host'
  else
    local extras = {
      addheaders = true,
      mode = 'smtpout',
      X_Sanitize = false,
    }

    local result = hornet:scan(hornethost, extras, msg)
    if result ~= nil then
      if result.error ~= nil then
        print('Hornetsecurity scan error: ' .. result.error)
      else
        --      print ("Hornetsecurity scan result :" .. result)
      end

      if result.verdict == 'malware' then
        kumo.reject(552, 'Bounced for detected malware')
      end
    end
  end

  -- rest of smtp_server_message_received processing code goes below
end)
```
