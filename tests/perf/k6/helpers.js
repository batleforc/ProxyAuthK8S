import { check } from "k6";
import http from "k6/http";

export function bearer(token) {
  return { headers: { Authorization: `Bearer ${token}`, Accept: "application/json" } };
}

// A GET that must answer 200 with a non-empty JSON body of `kind`.
export function getOk(url, params, kind) {
  const response = http.get(url, params);
  check(response, {
    "status is 200": (r) => r.status === 200,
    "body is the expected kind": (r) => {
      try {
        return !kind || r.json("kind") === kind;
      } catch (_) {
        return false;
      }
    },
  });
  return response;
}
