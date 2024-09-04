# `kumo.amqp.basic_publish {PARAMS}`

{{since('2024.09.02-c5476b89')}}

Publishes a message using the [amqprs](https://docs.rs/amqprs/) AMQP client.
This function differs from [kumo.amqp.build_client](build_client.md) in that it
encapsulates the entire publish operation in a single function that manages a
connection pool of client(s) and performs the publish without manually dealing
with the client connection lifetime.

*PARAMS* is an object style table with the following possible fields:

* `routing_key` - required string; the name of the queue to which to send the message
* `payload` - required string; the message to send
* `exchange` - optional string; the exchange through which to send the message. If unspecified,
  the empty string is used, which corresponds to a default exchange.
* `connection` - the connection information, which is in turn an object with the following fields:
    * `host` - required string; the hostname or IP address of the AMQP server
    * `port` - optional integer; the port number of the AMQP service.
    * `username` - optional string
    * `password` - optional string
    * `vhost` - optional string
    * `connection_name` - optional string
    * `heartbeat` - optional integer
    * `enable_tls` - optional boolean
    * `root_ca_cert` - optional string; the path to a CA certificate file.
    * `client_cert` - optional string; the path to a client certificate file.
    * `client_private_key` - optional string; the path to the private key associated with the client certificate.
    * `pool_size` - optional integer; specifies the maximum number of AMQP connections to pool. The default is the number of CPUs * 4.
    * `connect_timeout` - optional duration string; the timeout to use around the connect operation.
    * `recycle_timeout` - optional duration string; the timeout to use around the recycle operation, which is where the liveness of
      an existing connection is tested prior to reuse.
    * `wait_timeout` - optional duration string; the timeout to use while waiting for a fully saturated pool to have an available AMQP client.
    * `publish_timeout` - optional duration string; the timeout to use around the `basic_publish` operation.
* `app_id` - optional string
* `cluster_id` - optional string
* `content_encoding` - optional string
* `content_type` - optional string
* `correlation_id` - optional string
* `delivery_mode` - optional integer
* `expiration` - optional string
* `headers` - optional field table
* `message_id` - optional string
* `message_type` - optional string
* `priority` - optional integer
* `reply_to` - optional string
* `timestamp` - optional timestamp
* `user_id` - optional string
* `mandatory` - optional boolean
* `immediate` - optional boolean
