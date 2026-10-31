// The discovery kubectl makes before a command it has no cache for: /api,
// /apis and /version, in a row, as one iteration.
import { TOKEN, clusterUrl, options as base } from "../config.js";
import { bearer, getOk } from "../helpers.js";

export const options = base(["p(95)<300", "p(99)<800"]);

export default function () {
  const params = bearer(TOKEN);
  getOk(clusterUrl("/api"), params, "APIVersions");
  getOk(clusterUrl("/apis"), params, "APIGroupList");
  getOk(clusterUrl("/version"), params);
}
