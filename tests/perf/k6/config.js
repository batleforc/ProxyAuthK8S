// The k6 side of `task perf:http` (RFC 0016 §6.3): everything comes from the
// environment the harness (tests/integration/tests/perf_http.rs) sets, so a
// scenario never names a namespace, a token or a port itself.
//
// TLS: k6 trusts the run CA through SSL_CERT_FILE; the bench's host names
// resolve to 127.0.0.1 through `hosts`, as the suite's own client does.

export const BASE_URL = __ENV.BASE_URL;
export const HOST = __ENV.HOST || "proxyauth.k8s.localhost";
export const NS = __ENV.NS;
export const CLUSTER = __ENV.CLUSTER || "c";
export const TOKEN = __ENV.TOKEN;
export const DASHBOARD_TOKEN = __ENV.DASHBOARD_TOKEN;
// The hold stage of every scenario; `PERF_DURATION=4h` makes it a soak.
export const DURATION = __ENV.PERF_DURATION || "2m";
export const VUS = Number(__ENV.PERF_VUS || 10);
// Latency thresholds are targets: enforced only by `PERF_ENFORCE=1`. The
// error rate is enforced always.
const ENFORCE = __ENV.PERF_ENFORCE === "1";

export function clusterUrl(path) {
  return `${BASE_URL}/clusters/${NS}/${CLUSTER}${path}`;
}

// Ramp, hold, cool down; the hold is DURATION at VUS.
export function stages() {
  return [
    { duration: "20s", target: VUS },
    { duration: DURATION, target: VUS },
    { duration: "10s", target: 0 },
  ];
}

export function options(latency) {
  const thresholds = { http_req_failed: ["rate<0.01"] };
  if (ENFORCE) {
    thresholds.http_req_duration = latency;
  }
  return {
    stages: stages(),
    thresholds,
    hosts: { [HOST]: "127.0.0.1" },
    summaryTrendStats: ["avg", "med", "p(95)", "p(99)", "max"],
  };
}
