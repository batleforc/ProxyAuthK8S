### HTTP load (RFC 0016)

| Scenario | req/s | p95 ms | p99 ms | failed |
| --- | --- | --- | --- | --- |
| 01_proxied_get | 379.1 | 34.2 | 45.3 | 0.00 % |
| 02_proxied_list | 267.4 | 60.9 | 79.8 | 0.00 % |
| 03_discovery | 385.6 | 53.5 | 65.9 | 0.00 % |
| 04_dashboard_clusters | 1168.2 | 9.3 | 10.6 | 0.00 % |

Back memory (working set, MiB, every pod): before 4, peak 10, after 7.

Rate limit 120/min under 30 s of load: 120 accepted, 12590 refused with 429.
