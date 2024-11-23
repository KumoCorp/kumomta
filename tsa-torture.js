// This is a k6 script that can be used to test the tsa-daemon publish endpoint
import http from "k6/http";
import encoding from "k6/encoding";
import { check } from "k6";

export const options = {
  // How many concurrent connections should be established
  vus: 100,
  // How long the test should run for
  duration: '30s',
};

export default function () {
  const url = "http://127.0.0.1:8008/publish_log_v1";

  const content = {
    "type": "TransientFailure",
    "id": "1d98076abbbc11ed940250ebf67f93bd",
    "sender": "user@example.com",
    "recipient": "user@yahoo.com",
    "queue": "something",
    "site": "unspecified->(mta5|mta6|mta7).am0.yahoodns.net",
    "size": 1024,
    "response": {
      "code": 421,
      "enhanced_code": {
        "class": 4,
        "subject": 7,
        "detail": 0
      },
      "content": "[TSS04] Messages from a.b.c.d temporarily deferred due to user complaints",
      "command": "."
    },
    "timestamp": 1678069691,
    "created": 1678069691,
    "num_attempts": 1,
    "bounce_classification": "Uncategorized",
    "meta": {},
    "headers": {},
    "nodeid": "557f3ad4-2c8c-11ee-976e-782d7e12e173",
  }

  const payload = JSON.stringify(content);

  const params = {
    headers: {
      "Content-Type": "application/json",
    },
  };

  let response = http.post(url, payload, params);
  // console.log(response);
  check(response, {
    "is status 200": (r) => r.status === 200,
  });
}
