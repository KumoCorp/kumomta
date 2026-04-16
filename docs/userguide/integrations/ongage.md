# Ongage SMTP integration  

## Introduction

[Ongage](https://www.ongage.com/) is an email marketing platform that allows you to create and manage content and campaigns then deliver them through your favourite sending engine. This integration describes how to use KumoMTA as the delivery engine for Ongage.

## Instructions

### Get KumoMTA 

Install KumoMTA as per the [installation instructions
here](../installation/overview.md).  Before finishing this step, you should
ensure that you have correctly set up DNS with a resolving sending domain, MX,
PTR, SPF, DKIM, etc.

As part of this process, you will be creating a DNS entry for
“bounce.yoursendingdomain”, as well as similar tracking and image domains so
be prepared to edit your DNS.

Ensure that you can inject mail using SMTP_Auth (Plain) with TLS. Remember to
add the access credentials and test it. See [this
page](../operation/smtpinjection.md) for more information on injection.

!!! note
    TLS is crucial to this process, so ensure you have a valid and tested
    certificate attached to your listener config. See
    [tls_certificate](../../reference/kumo/start_esmtp_listener/tls_certificate.md).

### Get Ongage 

Go to [Ongage.com](https://www.ongage.com/registration) and create an account.

Create a support ticket to request access to use the “Private SMTP” connector
and let them know you are using KumoMTA as the sending MTA.

[Here is some essential
reading](https://ongage.atlassian.net/wiki/spaces/HELP/pages/657817611/The+Ongage+Private+SMTP+Connector)
before you set up the Private SMTP connector: 

Once registered, you can add a vendor by selecting Vendors > My Connections in the left menu

![Select a vendor](../../assets/images/ongage_vendor_select.png)

Create a new connection and add “Private SMTP” as the connection type.

!!! note
    If you do not see this as an option, jump back up to step 4 and ask Ongage
    Support to add it for you.

Now fill in all the required fields and test.

* You can choose any username and password you like, KumoMTA has no preset or preference.  These credentials should match what you set above.
* Select PLAIN authentication Type
* Select YES for “Use TLS”
* The SMTP Port should match what your listener is listening on.  Remember to update your public firewall too.

If you have done everything right, you will see a positive notification that credentials were verified.

Pat yourself on the back then start using Ongage with KumoMTA.

