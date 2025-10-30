# Rspamd Spam filter

## Introduction

[Rspamd](https://rspamd.com/) is an open source email spam filtering tool that can be integrated with KumoMTA

## Instructions

### First things
Spam filtering engines can be complex and require an understanding or patern matching and email handling rule sets. 
Please read through the quickstart documentation FIRST before proceeding: [https://docs.rspamd.com/tutorials/quickstart/](https://docs.rspamd.com/tutorials/quickstart/)

### Get KumoMTA
 1) Install KumoMTA as per the installation instructions here [https://docs.kumomta.com/userguide/installation/overview/](https://docs.kumomta.com/userguide/installation/overview/)
    Before finishing this step, you should ensure that you have correctly set up DNS with a resolving sending domain, MX, PTR, SPF, DKIM, etc.

 2) Ensure that you can inject and deliver mail through KumoMTA.

 3) Add the following to your init.lua config:

In the top part of the config, before the init section, place this variable declaration: 

`local RSPAMD_URL = "http://localhost:11333/checkv2"`

Then in the `smtp_server_message_received` add code to pass the message to rspamd for evaluation:

```lua
kumo.on('smtp_server_message_received', function(msg)
  local request = kumo.http.build_client({}):post(RSPAMD_URL)
  request:body(msg:get_data())
  local response = request:send()
  if response:status_code() == 200 then
    local data = kumo.serde.json_parse(response:text())
    if data['score'] >= 15 then -- rspamd recommends rejecting the message
      kumo.reject(550, 'We do not send spam')
    end
  end
  -- the rest of your handler
end)
```
Note that you can modify the score threshold and reject message as needed.


### Get Rspamd

 3) Read and folllow the first 3 (THREE) steps in this guide: [https://docs.rspamd.com/tutorials/quickstart/](https://docs.rspamd.com/tutorials/quickstart/)
 - STOP when you hit step 4.  Do NOT install Postfix.

 4) Restart the rspand process with  `sudo systemctl restart rspamd`

 5) Continue the install and test process starting at step 5 (five) here: [https://docs.rspamd.com/tutorials/quickstart/](https://docs.rspamd.com/tutorials/quickstart/)
 

Your rspamd configuration should now test every mesage injected to KumoMTA.



