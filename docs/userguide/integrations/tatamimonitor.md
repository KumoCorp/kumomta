# Tatami Monitor integration

## Introduction

[Tatami Monitor](https://tatamimonitor.com/) is an email monitoring and alerting platform that brings real-time insights to your email infrastructure. This integration describes how to us
e Tatami Monitor with KumoMTA.

## Instructions

### Get KumoMTA

 1) Install KumoMTA as per the installation instructions here
https://docs.kumomta.com/userguide/installation/overview/

Before finishing this step, you should ensure that you have correctly set up DNS with a resolving sending domain, MX, PTR, SPF, DKIM, etc.

 2) Ensure that you can inject and deliver mail through KumoMTA.


### Get connected to Tatami Monitor

 3) Go to [https://tatamimonitor.com/](https://tatamimonitor.com/) and create an account by clicking the "Sign Up" button.

 4) Tatami will send you back a webhook key.  Add a new log-hook to your init.lua right before the init section like this:
```lua
log_hooks:new_json {
  name = 'webhook_tatami',
  url = 'https://tatamimonitor.com/api/v1/webhooks/events/your_webhook_key_goes_here',
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
}
```

5) If you need to, you can modify the `log_parameters` as per [https://docs.kumomta.com/reference/kumo/configure_log_hook/](https://docs.kumomta.com/reference/kumo/configure_log_hook/)

If you have done everything right, you should see your data fill the Tatami Monitor feed almost immediately.



