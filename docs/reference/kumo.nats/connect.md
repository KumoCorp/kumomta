# kumo.nats.connect
{{since('dev')}}

```lua
kumo.nats.connect(CONFIG)
```

Connects to a NATS JetStream instance and constructs a client object.

`CONFIG` contains at least one address to connect to and the name. Supports password and token authentication. The following Parameters are available:

| Parameter                 | Description                                                                                                                                                                                                                                                                                                                 |
|---------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| servers                   | list of addresses                                                                                                                                                                                                                                                                                                           |
| auth                      | password or token                                                                                                                                                                                                                                                                                                           |
| name                      | Sets the name for the client.                                                                                                                                                                                                                                                                                               |
| no_echo                   | disables delivering messages that were published from the same connection                                                                                                                                                                                                                                                   |
| max_reconnects            | Specifies the number of consecutive reconnect attempts the client will make before giving up. This is useful for preventing zombie services from endlessly reaching the servers, but it can also be a footgun and surprise for users who do not expect that the client can give up entirely. Pass 0 for no limit (default). |
| connection_timeout        | Sets a timeout for the underlying TcpStream connection to avoid hangs and deadlocks. Default is set to 5 seconds                                                                                                                                                                                                            |
| tls_required              | Sets or disables TLS requirement. If TLS connection is impossible connection will return error                                                                                                                                                                                                                              |
| tls_first                 | Changes how tls connection is established. If set, client will try to establish tls before getting info from the server. That requires the server to enable handshake_first option in the config                                                                                                                            |
| certificate               | Loads root certificates by providing the path to them                                                                                                                                                                                                                                                                       |
| client_cert               | Loads client certificate by providing the path to it (client_key must be set)                                                                                                                                                                                                                                               |
| client_key                | Loads client key by providing the path to it (client_cert must be set)                                                                                                                                                                                                                                                      |
| ping_interval             | Sets how often Client sends PING message to the server                                                                                                                                                                                                                                                                      |
| sender_capacity           | By default, Client dispatches op’s to the Client onto the channel with capacity of 128. This option enables overriding it                                                                                                                                                                                                   |
| inbox_prefix              | Sets custom prefix instead of default _INBOX                                                                                                                                                                                                                                                                                |
| request_timeout           | Sets a timeout for requests. Default value is set to 10 seconds                                                                                                                                                                                                                                                             |
| retry_on_initial_connect  | By default, connect will return an error if the connection to the server cannot be established. Setting retry_on_initial_connect makes the client establish the connection in the background                                                                                                                                |
| ignore_discovered_servers | By default, a server may advertise other servers in the cluster known to it. By setting this option, the client will ignore the advertised servers. This may be useful if the client may not be able to reach them                                                                                                          |
| retain_servers_order      | By default, client will pick random server to which it will try connect to. This option disables that feature, forcing it to always respect the order in which server addresses were passed                                                                                                                                 |

Authentication with username and password
```lua
local nats = kumo.nats.connect {
  servers = {'127.0.0.1:4222'},
  -- optional arguments for authentication and connection behavior
  name = 'nats-client',
  auth = {
    username = 'username',
    password = 'password',
  }
}
```

Authentication with token
```lua
local nats = kumo.nats.connect {
  servers = {'127.0.0.1:4222', '127.0.0.1:4422'},
  -- optional arguments for authentication and connection behavior
  name = 'nats-client',
  auth = {
    token = 'token',
  }
}
```
