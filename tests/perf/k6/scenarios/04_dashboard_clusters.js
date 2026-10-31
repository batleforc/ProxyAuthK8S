// The dashboard's cluster list (/api/v1/clusters) with a dashboard token:
// the proxy's own API, no upstream behind it.
import { BASE_URL, DASHBOARD_TOKEN, options as base } from "../config.js";
import { bearer, getOk } from "../helpers.js";

export const options = base(["p(95)<200", "p(99)<500"]);

export default function () {
  getOk(`${BASE_URL}/api/v1/clusters`, bearer(DASHBOARD_TOKEN));
}
