# Cancelling Queued Messages

Occasionally, you will have a bad send or for some reason need to cancel a send
quickly. OOPS! It happens.  KumoMTA offers an API specifically for
administratively canceling messages with surgical precision. The [Admin
Bounce](../../reference/http/kumod/api_admin_bounce_v1_post.md) API can
target a specific Campaign, Queue, or entire Tenant for cancellation.

You easiest way to invoke this API is via the [kcli
bounce](../../reference/kcli/bounce.md) utility, but you can also use `curl`
directly.

!!! warning
    There is no way to undo the actions carried out by this request!

You can set a time period that you want the bounce to last for; default is 5
minutes.  This can be handy if you need to catch all new injections for the
next hour and you don't want to keep running the command.

This will bounce everything and is not reversible - handle with care: 

=== "kcli"
    ```console
    $ /opt/kumomta/sbin/kcli bounce --everything \
          --reason 'PURGING ALL THE QUEUES!'
    ```

=== "curl"
    ```console
    $ curl -i 'http://localhost:8000/api/admin/bounce/v1' \
         -H 'Content-Type: application/json' \
         -d '{"reason":"PURGING ALL THE QUEUES!"}'
    ```

This will bounce all mail destined to yahoo.com:

=== "kcli"
    ```console
    $ /opt/kumomta/sbin/kcli bounce --domain yahoo.com \
           --reason 'felt like it'
    ```

=== "curl"
    ```console
    $ curl -i 'http://localhost:8000/api/admin/bounce/v1' \
          -H 'Content-Type: application/json' \
          -d '{"domain": "yahoo.com", "reason":"felt like it"}'
    ```

This will bounce all mail to any domain in the campaign “Back to school” 

=== "kcli"
    ```console
    $ /opt/kumomta/sbin/kcli bounce --campaign 'Back to school' \
            --reason 'felt like it'
    ```

=== "curl"
    ```console
    $ curl -i 'http://localhost:8000/api/admin/bounce/v1' \
           -H 'Content-Type: application/json' \
           -d '{"campaign": "Back to school", "reason":"felt like it"}'
    ```

NOTE: All fields are case-sensitive. However, domain names are normalized to
lowercase when a message is queued, so our internal queue names are always
built from the lower cased domain name.


