# `GET /metrics.json`

Exports various counters, gauges and other metrics in a JSON format loosely
derived from the [Prometheus Text
Exposition
Format](https://prometheus.io/docs/instrumenting/exposition_formats/).

Access to this endpoint requires *Trusted IP* authentication. HTTP
authentication is not permitted.

See also [metrics](metrics.md).

## Example data

Here's an example of the shape of the data. The precise set of counters
will vary as we continue to enhance KumoMTA.

```json
{
  "connection_count": {
    "help": "number of active connections",
    "type": "gauge",
    "value": {
      "service": {
        "esmtp_listener": 0.0,
        "smtp_client": 0.0,
        "smtp_client:source2->": 0.0
      }
    }
  },
  "scheduled_count": {
    "help": "number of messages in the scheduled queue",
    "type": "gauge",
    "value": {
      "queue": {
        "example.com": 0.0
      }
    }
  },
  "lua_count": {
    "help": "the number of lua contexts currently alive",
    "type": "gauge",
    "value": 1.0
  },
  "lua_load_count": {
    "help": "how many times the policy lua script has been loaded into a new context",
    "type": "counter",
    "value": 1.0
  },
  "lua_spare_count": {
    "help": "the number of lua contexts available for reuse in the pool",
    "type": "gauge",
    "value": 1.0
  },
  "memory_limit": {
    "help": "soft memory limit measured in bytes",
    "type": "gauge",
    "value": 101234377728.0
  },
  "memory_usage": {
    "help": "number of bytes of used memory",
    "type": "gauge",
    "value": 185683968.0
  },
  "message_count": {
    "help": "total number of Message objects",
    "type": "gauge",
    "value": 1.0
  },
  "message_data_resident_count": {
    "help": "total number of Message objects with body data loaded",
    "type": "gauge",
    "value": 1.0
  },
  "message_meta_resident_count": {
    "help": "total number of Message objects with metadata loaded",
    "type": "gauge",
    "value": 1.0
  },
  "ready_count": {
    "help": "number of messages in the ready queue",
    "type": "gauge",
    "value": {
      "service": {
        "smtp_client:source1->loopback.dummy-mx.example.com": 46.0,
        "smtp_client:source2->loopback.dummy-mx.example.com": 152.0,
      }
    }
  },
  "total_connection_count": {
    "help": "total number of active connections ever made",
    "type": "counter",
    "value": {
      "service": {
        "smtp_client": 0.0,
        "smtp_client:source2->": 0.0
      }
    }
  },
  "total_messages_delivered": {
    "help": "total number of messages ever delivered",
    "type": "counter",
    "value": {
      "service": {
        "smtp_client": 0.0,
        "smtp_client:source2->": 0.0
      }
    }
  },
  "total_messages_fail": {
    "help": "total number of message delivery attempts that permanently failed",
    "type": "counter",
    "value": {
      "service": {
        "smtp_client": 0.0,
        "smtp_client:source2->": 0.0
      }
    }
  },
  "total_messages_transfail": {
    "help": "total number of message delivery attempts that transiently failed",
    "type": "counter",
    "value": {
      "service": {
        "smtp_client": 0.0,
        "smtp_client:source2->": 0.0
      }
    }
  }
}
```
