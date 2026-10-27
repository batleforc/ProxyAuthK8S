# Security Policy

## Reporting a vulnerability

Report it **privately**, through GitHub's private vulnerability reporting:

**<https://github.com/batleforc/ProxyAuthK8S/security/advisories/new>**

Please do not report a vulnerability in a public issue, a discussion, a pull
request or a chat channel: anything filed there is visible to everyone the
moment it is posted, before a fix exists.

Include, as far as you can: what is affected (component, version or commit),
the impact and the conditions it needs, steps to reproduce or a proof of
concept, and whether and when you intend to publish.

## What happens next

These are **targets**, not a contractual SLA.

| Step | Target |
| --- | --- |
| Acknowledgement of your report | 5 working days |
| First assessment: confirmed or not, and a severity (CVSS) | 10 working days |
| Fix released — critical | 30 days from confirmation |
| Fix released — high | 60 days from confirmation |
| Fix released — medium and low | the next scheduled release |

## Disclosure

Disclosure is coordinated. The advisory is published when the fixed release
ships, or **90 days after the report** if no fix has shipped by then —
earlier if the vulnerability is being actively exploited, later only by
agreement with you.

Every published vulnerability gets, in the same release: a **GitHub Security
Advisory** with a **CVE** requested through GitHub; and an entry under
`### Security` in `CHANGELOG.md`.

## Supported versions

| Version | Security fixes |
| --- | --- |
| The latest published release | Yes |
| Any earlier release | No |

## Scope

Out of scope: a deployment's own configuration (unless our defaults or
documentation led to it), volumetric denial of service, and output of an
automated scanner with no demonstrated impact.

## Safe harbour

Research done in good faith — within scope, without accessing or modifying
other people's data, without degrading a service for others, and reported
through the channel above — will not be the subject of a complaint from the
project. Test against your own instance.
