// One object read through the proxy (Passthrough, token validated, relayed
// to the bench's apiserver): the request every kubectl get pays.
// Run: `task perf:http` (all scenarios, report only).
import { BASE_URL, NS, TOKEN, clusterUrl, options as base } from "../config.js";
import { bearer, getOk } from "../helpers.js";

export const options = base(["p(95)<300", "p(99)<800"]);

export default function () {
  getOk(clusterUrl(`/api/v1/namespaces/${NS}/configmaps/app`), bearer(TOKEN), "ConfigMap");
}

export function setup() {
  if (!BASE_URL || !TOKEN) throw new Error("BASE_URL and TOKEN come from the harness");
}
