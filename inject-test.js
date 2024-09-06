// This is a k6 script that can be used to test the kumomta http
// injection API endpoint.
import http from "k6/http";
import encoding from "k6/encoding";
import { check } from "k6";

// Whether to use the builder mode of the API, which is where the request
// defines the various parts and attachments and defers to kumomta to
// build the actual MIME message.  This convenience costs CPUs on the
// kumomta node, generally cutting injection rate by about half when
// using it.
const use_builder = true;
// The number of recipients to include in any given request
const batch_size = 100;
// Whether deferred_spool should be used.
// Setting deferred_spool=true can boost performance but comes with
// the risk of the loss of accountability if the service crashes
const deferred_spool = false;
// Whether deferred_generation should be used.
// This causes the generation to complete asynchronously wrt. the
// injection request.
const deferred_generation = true;
// Whether the client should compress the request body.
// KumoMTA does not currently support this on the server side,
// so enabling this will fail
const compression = false;
// How large the generated messages should be
const payload_size = 100000;

export const options = {
  // How many concurrent connections should be established
  vus: 40,
  // How long the test should run for
  duration: '30s',
};

// ---- there's no easy configuration beyond this point

const nonsense = "akjhasdlfkjhl kajsdhfl kjh lkajsdhflkjh asdkjfh asdf\n";
const num_lines = (payload_size / 2) / nonsense.length;
const lines = nonsense.repeat(num_lines);
const big_lines = nonsense.repeat(num_lines * 2);

export default function () {
  // const credentials = "scott:tiger";
  // const encodedCredentials = encoding.b64encode(credentials);
  const url = "http://127.0.0.1:8000/api/inject/v1";

  const content = use_builder ?  {
      text_body: lines,
      html_body: lines,
      from: {
        email: "info@testing.mx-sink.wezfurlong.org",
        name: "Test",
      },
      subject: "This is the subject from k6",
      reply_to: {
        email: "reply@testing.mx-sink.wezfurlong.org",
        name: "k6-test",
      },
    } : `Subject: woot\nFrom: wez@testing.mx-sink.wezfurlong.org\n\n{big_lines}`;

  var recipients = [];
  for (var i = 0; i < batch_size; i++) {
    var rand = Math.floor(Math.random() * 100000000) + 1; // get b between 1 and 1000
    if (rand < 70000000) {
      var domain = "gmail.com.mx-sink.wezfurlong.org";
    } else if (rand > 70000000 && rand < 90000000) {
      var domain = "yahoo.com.mx-sink.wezfurlong.org";
    } else {
      var domain = "aol.com.mx-sink.wezfurlong.org";
    }
    recipients.push({email: `someone-${i}@${domain}`});
  }

  const payload = JSON.stringify({
    envelope_sender: "sender@testing.mx-sink.wezfurlong.org",
    content: content,
    recipients: recipients,
    deferred_spool: deferred_spool,
    deferred_generation: deferred_generation,
  });

  const params = {
    headers: {
      "Content-Type": "application/json",
      // Authorization: `Basic ${encodedCredentials}`,
      // "Accept-Encoding": "deflate",
    },
    compression: compression ? "deflate" : "",
  };

  let response = http.post(url, payload, params);
  // console.log(response);
  check(response, {
    "is status 200": (r) => r.status === 200,
    "success_count present in output": (r) => r.json().success_count >= 0,
  });
}
