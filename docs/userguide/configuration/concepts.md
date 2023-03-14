# Configuration Concepts

KumoMTA differs from existing commercial and Open Source MTAs in that there is no configuration file in the traditional sense. Instead, all server configuration is achieved through the passing of a policy file written in Lua at server startup.

At first, configuration using a policy script may seem like a departure from the traditional approach to configuration, but using a Lua script as a configuration methodology will look quite familiar to administrators of the popular commercial MTA solutions.

Take a look at the [example policy](example.md) to see how a configuration policy approach can be quite similar to a traditional configuration file.
