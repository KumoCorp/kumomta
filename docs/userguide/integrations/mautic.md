# Mautic integration

## Introduction

[Mautic](https://mautic.org/) is an open source  marketing automation project that drives email campaigns. This page explains how to use Mautic with KumoMTA.

## Instructions

### Get KumoMTA

 1) Install KumoMTA as per the installation instructions here
https://docs.kumomta.com/userguide/installation/overview/

Before finishing this step, you should ensure that you have correctly set up DNS with a resolving sending domain, MX, PTR, SPF, DKIM, etc.

 2) Ensure that you can inject and deliver mail through KumoMTA.

 3) Mautic will require an SMTP_Auth authentication connection to inject to KumoMTA, so you will also need to provide KumoMTA with a way to validate credentials.  There is an explanation and sample code on [this page](../../reference/events/smtp_server_auth_plain/) that will allow you to store valid user credentials in KumoMTA.  

Familiarize yourself with the documentation [here](..//operation/smtpinjection/) on SMTP Injection.

### Got Mautic already?
[Skip to part 5](#part_5)


### Get Mautic

 3) Go to [https://mautic.org/](https://mautic.org/) and explore the project and your options;  There are many.
    You can [download the production zip](https://mautic.org/download/) or you can [checkout the GitHub repo](https://github.com/mautic/mautic) depending on how much you want to customize and contribute.

 4) Follow the install instructions [here](https://docs.mautic.org/en/5.x/getting_started/how_to_install_mautic.html) 

<a name="part_5" />
 5) KumoMTA accepts messages for delivery with SMTP_Auth Plain authentication.  This is the default for Mautic, but you may need to make specific edits based on their Symfony Mailer ingtegration so the DSN looks like `smtp://user:pass@smtp.example.com:port`. See [this](https://docs.mautic.org/en/5.x/configuration/settings.html#smtp-transport) for more detail.

 6) If you already have Mautic version 4 or earlier, the next step may be as easy as completing the configuration [here](https://docs.mautic.org/en/5.x/getting_started/how_to_install_mautic.html#configuring-email-settings)

If you have Mautic version 5 or newer, then the above is *likely* still true, but you may need to configure settings to specify the DSN as described [here](https://docs.mautic.org/en/5.x/configuration/settings.html#smtp-transport) 


If you have done everything right, you should now be able to send messages from Mautic through KumoMTA with SMTP.



