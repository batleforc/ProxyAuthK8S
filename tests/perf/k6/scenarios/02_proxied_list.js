// A list of ~50 ConfigMaps through the proxy: the body is streamed back, so
// this is where a buffering regression shows.
import { NS, TOKEN, clusterUrl, options as base } from "../config.js";
import { bearer, getOk } from "../helpers.js";

export const options = base(["p(95)<500", "p(99)<1200"]);

export default function () {
  getOk(clusterUrl(`/api/v1/namespaces/${NS}/configmaps`), bearer(TOKEN), "ConfigMapList");
}
