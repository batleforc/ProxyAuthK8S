// A cluster limited to RATE_LIMIT requests per minute, hammered by VUS for
// 30 s (inside one window): `accepted` must stay at or under the limit and
// `limited` must be above zero. The harness reads both counters from the
// summary; a status other than 200 or 429 is a failure.
import { check } from "k6";
import http from "k6/http";
import { Counter } from "k6/metrics";
import { HOST, NS, TOKEN, VUS, clusterUrl } from "../config.js";
import { bearer } from "../helpers.js";

const accepted = new Counter("accepted");
const limited = new Counter("limited");

http.setResponseCallback(http.expectedStatuses(200, 429));

export const options = {
  scenarios: {
    burst: { executor: "constant-vus", vus: VUS, duration: "30s" },
  },
  thresholds: { http_req_failed: ["rate<0.01"], limited: ["count>0"] },
  hosts: { [HOST]: "127.0.0.1" },
};

export default function () {
  const response = http.get(clusterUrl(`/api/v1/namespaces/${NS}/configmaps/app`), bearer(TOKEN));
  check(response, { "200 or 429": (r) => r.status === 200 || r.status === 429 });
  if (response.status === 200) accepted.add(1);
  if (response.status === 429) limited.add(1);
}
