# Chapter 2 - Beyond The Basics

Now that you have Kumo MTA installed and sending email, it is time to look at refining the configuration.  Just being able to send a single test email is nice, but if you plan to send any volume of mail, you need to consider many factors like resilliency, reporting, and security. 

The samples below are present in the _*simple_policy.lua*_ file included with the default build.  You can add to or replace this entirely with your own config.  The config is entirely written in Lua which reads like english, but has the power of C.

## Control Access

First let's make sure only authorized systems can access your MTA.  For SMTP, this is done in the configuration with relay_hosts:
```  -- override the default set of relay hosts
    relay_hosts = { '127.0.0.1', '192.168.1.0/24' },
```
By default only localhost and private networks are able to relay (send) mail.  Add the IP address or CIDR block of your injectors here to allow them to relay mail.

For HTTP, this is done with the _*trusted_hosts*_ setting in a litener stanza.
``` 
 kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }
```


