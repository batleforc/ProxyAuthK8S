# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.4.2](https://github.com/batleforc/proxyauthK8s/compare/6a904f78ae90d6334e3ab8f42e400c55b31c5bfc..v0.4.2) - 2026-10-28
#### Bug Fixes
- (**ci**) install mise's rust before cargo-cyclonedx in the release SBOM job - ([04628cc](https://github.com/batleforc/proxyauthK8s/commit/04628cce032508a210349f88fc148b5c448f4f5d)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- merge fix/release-sbom-rust (#317) - ([9c40211](https://github.com/batleforc/proxyauthK8s/commit/9c40211bb72f6e785ce39a7626ee46ed90a78376)) - Maxime
- merge release/v0.4.1 (#309) - ([6a904f7](https://github.com/batleforc/proxyauthK8s/commit/6a904f78ae90d6334e3ab8f42e400c55b31c5bfc)) - Maxime

- - -

## [v0.4.1](https://github.com/batleforc/proxyauthK8s/compare/9b2f3eb836dbfcee3837b11d1a2533d91e315192..v0.4.1) - 2026-10-27
#### Features
- (**agent**) negotiate the tunnel protocol, validate what agents announce (RFC 0013 phase 1) - ([a72bfc4](https://github.com/batleforc/proxyauthK8s/commit/a72bfc4dbe20075d760facdc9982eb01c304c861)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**controller**) show agent versions in the status, warn on skew, prove it on the bench (RFC 0013 phases 2 and 3) - ([01f878f](https://github.com/batleforc/proxyauthK8s/commit/01f878fafe0b3a0faeb0ed05e0589256c548605b)) - Max Batleforc, Claude Opus 5.5 (1M context)
- apply agent security and prepare new RFC 14 - ([7243f1f](https://github.com/batleforc/proxyauthK8s/commit/7243f1f084d395f6c70597f2156588d34230a5ab)) - Max Batleforc
#### Bug Fixes
- (**ci**) keep mise exec to the installed tools in deps.yaml - ([5d5e3b9](https://github.com/batleforc/proxyauthK8s/commit/5d5e3b932237b1be17520170f07afc27a90085f5)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**ci**) install rust through mise before cargo-machete - ([f540b49](https://github.com/batleforc/proxyauthK8s/commit/f540b496e1a3c0bc45115d170e7f081e92bf148a)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**ci**) install uv before semgrep, rely on CodeQL default setup - ([14902fc](https://github.com/batleforc/proxyauthK8s/commit/14902fc8cb37b00051210237516b1cb6c05e1314)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**crd**) send what status.agent no longer reports as null - ([074b169](https://github.com/batleforc/proxyauthK8s/commit/074b169255c1879a73d709112f4db60b4a7b1e24)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**test**) scrape every agent gateway pod in RFC 0012 p15 - ([6ea9c58](https://github.com/batleforc/proxyauthK8s/commit/6ea9c5890544c7cb084e44f3901ebfa3218e80df)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Documentation
- (**rfc**) add RFC 0013, agent versions and protocol negotiation, in review - ([c4a4783](https://github.com/batleforc/proxyauthK8s/commit/c4a4783436dfc43e4948295d04a7941f9a806586)) - Max Batleforc, Claude Opus 5.5 (1M context)
- migration guides, the 0.4.0 one first - ([c7f3906](https://github.com/batleforc/proxyauthK8s/commit/c7f3906b43db1c2a5d6cc08800cd1066d39ff0d7)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Tests
- (**integration**) RFC 0013 p5 waits out the refusal key and a requeue - ([c79feb8](https://github.com/batleforc/proxyauthK8s/commit/c79feb872c525fc7b886f79064baa16a53db1719)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) give RFC 0013 p5 10 min, it waits out the refusal key - ([7cf6674](https://github.com/batleforc/proxyauthK8s/commit/7cf6674cfd2946eb4d97074d18a502f10e0c3abc)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) drop the branch agent before the refused one (RFC 0013 p5) - ([42a3f48](https://github.com/batleforc/proxyauthK8s/commit/42a3f4881f747553fefb06dbc5e049c0efec36cd)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Build system
- (**deps**) bump taiki-e/install-action from 2.87.13 to 2.87.29 - ([98cc1c2](https://github.com/batleforc/proxyauthK8s/commit/98cc1c2ddafc23ff61d6e813a07593ff014066af)) - dependabot[bot]
- (**deps**) bump actions/upload-artifact from 4.6.2 to 7.0.2 - ([19a48fe](https://github.com/batleforc/proxyauthK8s/commit/19a48fe121e98214d4454fd80991c9bd88e631db)) - dependabot[bot]
#### Continuous Integration
- (**security**) add the weebo-security gates, SBOMs and attestations - ([1afe006](https://github.com/batleforc/proxyauthK8s/commit/1afe006ae0004efe774bcd52e11a0764b62202db)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- (**ci**) drop the retired workflows, let cog skip merge commits - ([b43f88e](https://github.com/batleforc/proxyauthK8s/commit/b43f88eff60ac3fd1babb02d57844194de6da4c9)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**deps**) hold tokio-tungstenite at 0.29, from the workspace - ([5c40925](https://github.com/batleforc/proxyauthK8s/commit/5c40925fbc2698588db66241a505794838d2b6aa)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**deps**) bump everything within its range - ([df611ce](https://github.com/batleforc/proxyauthK8s/commit/df611cea03294ee705bc0fb553f3f2d54543ea1f)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**deps**) drop @lucide/vue and axios from the front - ([e3de1c5](https://github.com/batleforc/proxyauthK8s/commit/e3de1c538059d9a0c6a67004c9f3f1c2122cfc43)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**deps**) add the task deps checks and knip - ([e7513dd](https://github.com/batleforc/proxyauthK8s/commit/e7513dd510a5f161e21e2cbc4373ae244a1fb7f6)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**tooling**) stop tracking the wska skill links - ([77ecd9d](https://github.com/batleforc/proxyauthK8s/commit/77ecd9d588472adcdbefdaa36284f58e56b5a591)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**tooling**) move to weebo-skillsagent 0.17.0 and graphify 0.9.96 - ([9e43e64](https://github.com/batleforc/proxyauthK8s/commit/9e43e64fc059598bbc0af22edad3adeacceb9cc8)) - Max Batleforc, Claude Opus 5.5 (1M context)
- merge dependabot/github_actions/taiki-e/install-action-2.87.30 (#299) - ([3712b7e](https://github.com/batleforc/proxyauthK8s/commit/3712b7e2f8c98fa005eedd3af923940d7d4a525a)) - Maxime
- merge chore/drop-retired-workflows (#308) - ([407d34c](https://github.com/batleforc/proxyauthK8s/commit/407d34c3060e4035cce8d166dab9c8a906f8d415)) - Maxime
- merge chore/deps-hygiene (#307) - ([9036cc8](https://github.com/batleforc/proxyauthK8s/commit/9036cc8def71b265546baeedc091b1be30300b92)) - Maxime
- merge docs/rfc-0013-agent-versions (#304) - ([c565025](https://github.com/batleforc/proxyauthK8s/commit/c5650254ffed834cb2dfd8ed64320df17f70bda5)) - Maxime
- merge chore/local-tooling (#306) - ([963b5a9](https://github.com/batleforc/proxyauthK8s/commit/963b5a98cf086f0f8fb999fb54dd884b47929783)) - Maxime
- merge ci/security-gates (#305) - ([99ce856](https://github.com/batleforc/proxyauthK8s/commit/99ce856c7004cdff1f5bfbb77aa7d88a99b7b6b0)) - Maxime
- merge docs/migration-guides (#302) - ([d7f10c9](https://github.com/batleforc/proxyauthK8s/commit/d7f10c92dd0be43e19dba8052a3f7a4f9cf6b66f)) - Maxime
- merge release/v0.4.0 (#301) - ([9b2f3eb](https://github.com/batleforc/proxyauthK8s/commit/9b2f3eb836dbfcee3837b11d1a2533d91e315192)) - Maxime

- - -

## [v0.4.0](https://github.com/batleforc/proxyauthK8s/compare/01c762c3f4af03e96d330217697539771b468538..v0.4.0) - 2026-10-25
#### Features
- (**agent**) jwksPublisher CronJob, the remote JWKS kept current (RFC 0012) - ([d8c8d37](https://github.com/batleforc/proxyauthK8s/commit/d8c8d37e482d5fe97026cd2502317a3d0e855cd3)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**agent**) OneShotToken bootstrap with a shared identity Secret (RFC 0012) - ([5a926d9](https://github.com/batleforc/proxyauthK8s/commit/5a926d996db02365e5e8d5f875b3a71458b87d49)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**agent**) egress through a CONNECT or SOCKS5 proxy (RFC 0012) - ([0cd92ea](https://github.com/batleforc/proxyauthK8s/commit/0cd92eaa8d1bb3fdab65f18668627aaf8d1aaac3)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**agent**) add the agent binary, its image and its chart (RFC 0012) - ([1feb4cf](https://github.com/batleforc/proxyauthK8s/commit/1feb4cfe9f20a53fcdd9c3c34b118709befb0f24)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**agent**) add the agent CA crate (RFC 0012, phase 1c) - ([20172e2](https://github.com/batleforc/proxyauthK8s/commit/20172e20c6dafa44beca8eb2b74e6d6238c3c1af)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**agent**) add the tunnel wire protocol crate (RFC 0012, phase 1b) - ([cac4528](https://github.com/batleforc/proxyauthK8s/commit/cac45283cb42921a66b1d487361211941948d750)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) ExternalCA bootstrap, agents certified by your own PKI (RFC 0012) - ([6c5bdcd](https://github.com/batleforc/proxyauthK8s/commit/6c5bdcd200fd30b68fda467f782e4f48d3d96bd9)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) agent gateway in tlsMode passthrough (RFC 0012) - ([21d249d](https://github.com/batleforc/proxyauthK8s/commit/21d249d490a7331742f0053b5166375214bd3b12)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) audit and measure the agent gateway (RFC 0012) - ([5a54b43](https://github.com/batleforc/proxyauthK8s/commit/5a54b43209e2f08dba3525e93174a3149dc64572)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) validate tokens against an Agent cluster's apiserver (RFC 0012) - ([3286539](https://github.com/batleforc/proxyauthK8s/commit/3286539db84896ce2ef27c946ed0cf14d3c45103)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) publish gateway presences and revoke stale tunnels (RFC 0012) - ([184eb11](https://github.com/batleforc/proxyauthK8s/commit/184eb11e6ed89d8febf38a25a81dd8a12c3984d0)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) enrol agents with a ServiceAccount token (RFC 0012) - ([91161b0](https://github.com/batleforc/proxyauthK8s/commit/91161b005b516394e7811d73a02463ffd66e18fe)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) accept agent tunnels on the gateway's listener (RFC 0012) - ([f092cf2](https://github.com/batleforc/proxyauthK8s/commit/f092cf2573da0df224e3d6f08b388c3f99c35b42)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) create the agent CA Secret from the gateway (RFC 0012) - ([3ad574c](https://github.com/batleforc/proxyauthK8s/commit/3ad574c9b9759162d6c3044de4826408822c0991)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) send Agent clusters through the agent gateway (RFC 0012) - ([d5fe653](https://github.com/batleforc/proxyauthK8s/commit/d5fe6537ce3b712ba2607b089479b5daa8b87733)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) add the agent gateway's tunnel registry and CONNECT (RFC 0012) - ([cfaf6a0](https://github.com/batleforc/proxyauthK8s/commit/cfaf6a053f6d35d9da8500e4fa1bcc42528a1211)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**chart**) deploy the agent gateway (RFC 0012) - ([ca0edb9](https://github.com/batleforc/proxyauthK8s/commit/ca0edb93430d5decfd57f5551797ba525ad00a54)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**controller**) OneShotToken bootstrap, written and consumed once (RFC 0012) - ([7419aac](https://github.com/batleforc/proxyauthK8s/commit/7419aacdb34e22f0c59983b2686bddc4046c259e)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**controller**) report agent tunnels on the ProxyKubeApi (RFC 0012) - ([6df11fe](https://github.com/batleforc/proxyauthK8s/commit/6df11fef097a8098cae87ede2ae3a4a12e71bae8)) - Max Batleforc, Claude Opus 5.5 (1M context)
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) (**controller**) adopt weebo-operator-kit's runtime, election, perf and lints (RFC 0011) - ([0c0e9ec](https://github.com/batleforc/proxyauthK8s/commit/0c0e9ecb97cc1350e57a9f468cea08a7773cec12)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**crd**) add the Agent service variant, closed until a tunnel serves it - ([711c02c](https://github.com/batleforc/proxyauthK8s/commit/711c02c66077584f4420eaa7aceb27900b73d7a2)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**server**) run the agent gateway by PROXYAUTH_ROLE (RFC 0012) - ([8b34416](https://github.com/batleforc/proxyauthK8s/commit/8b344160ce839eace7aa3ef8373137be88ead888)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Bug Fixes
- (**api**) close RFC 0012 gaps, JWKS size bound and gateway operability - ([90bc465](https://github.com/batleforc/proxyauthK8s/commit/90bc465f0ca9ff24e826555658edd24c1aab15aa)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) reach gateway pods by EndpointSlice, and 503 an untunnelled review (RFC 0012) - ([c79f579](https://github.com/batleforc/proxyauthK8s/commit/c79f5791a7829c135cb27be5fd7b15e1b05fd994)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**chart**) PDB, HPA and metrics netpol, configurable JWKS bound - ([bed65b7](https://github.com/batleforc/proxyauthK8s/commit/bed65b77fe9424fd393b398514f9ba4a649de155)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**deps**) accept RUSTSEC-2026-0387, bump fast-uri to 3.1.9 - ([7b0ccd9](https://github.com/batleforc/proxyauthK8s/commit/7b0ccd956525a337605746e91ee13b18a77d3ece)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**docs**) show the reconcile alert as text, Shiki has no promql - ([2beac8a](https://github.com/batleforc/proxyauthK8s/commit/2beac8aa6d074214a63deaa3e5589381273fb056)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Documentation
- (**rfc**) RFC 0012 implemented, phases 1 to 4 and gaps merged - ([c623e4b](https://github.com/batleforc/proxyauthK8s/commit/c623e4b0fc3fc6f515074d62603affe3fc74c3de)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0012 resume notes after PR #294 - ([1ed52fc](https://github.com/batleforc/proxyauthK8s/commit/1ed52fceb6769ef8ec440904c2f205309906a22d)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) name PR #294 in RFC 0012 status - ([0f6ce9b](https://github.com/batleforc/proxyauthK8s/commit/0f6ce9b0775dd81b35da4dd8a62d31b3a636aeb5)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) land RFC 0012 phase 1 - ([13d6f13](https://github.com/batleforc/proxyauthK8s/commit/13d6f135b0c46fc1581882a5f95fd9b783e04125)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) 0012 decisions 25 to 27, an agent gateway reached by CONNECT - ([72dde82](https://github.com/batleforc/proxyauthK8s/commit/72dde8244e622616caf21973372315a47fa290e7)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) 0012 decision 24, JWKS from a ConfigMap or Secret - ([4571631](https://github.com/batleforc/proxyauthK8s/commit/4571631d7c38027588e448e22da9817af6f1d6cf)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) accept RFC 0012, ship it with 0011 in 0.4.0, 0010 in 0.5.0 - ([114bf23](https://github.com/batleforc/proxyauthK8s/commit/114bf237e41a02089ac40cd25fc820c3da408ad3)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) land RFC 0011, Implemented - ([6304316](https://github.com/batleforc/proxyauthK8s/commit/6304316283824bfb7ab03544f29ce0da25320850)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) add RFC 0012, agent mode over an outbound tunnel, for 0.5.0 - ([14996c4](https://github.com/batleforc/proxyauthK8s/commit/14996c4db46ddb6700493fab33ea034dae57fea5)) - Max Batleforc, Claude Opus 5.5 (1M context)
- explain clusters reached through an agent (RFC 0012) - ([79181e5](https://github.com/batleforc/proxyauthK8s/commit/79181e59f271fa6a5a5351f2098a176656463e32)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Tests
- (**api**) drop a debug print CodeQL reads as key logging (RFC 0012) - ([843efdd](https://github.com/batleforc/proxyauthK8s/commit/843efdded9c02c13189f54bb4c0f49676053459d)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) prove the agent tunnel's threat model (RFC 0012) - ([36eadb0](https://github.com/batleforc/proxyauthK8s/commit/36eadb07bb705fb8bc21bd66d427c13d125ecd41)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**controller**) prove an OIDC config error never shows the client secret - ([808ca76](https://github.com/batleforc/proxyauthK8s/commit/808ca76d5492a130229972725e4ac9a6c7177a01)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) RFC 0012 phase 2 on the bench, points 8 to 11 - ([372410e](https://github.com/batleforc/proxyauthK8s/commit/372410e82798257d248a9f4bd0c4cdac55d91f9f)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) prove RFC 0012 on the bench - ([c745420](https://github.com/batleforc/proxyauthK8s/commit/c745420356941a89a7d01438c3ce8b880f678090)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Continuous Integration
- (**integration**) raise the job timeout to 45 minutes - ([01c762c](https://github.com/batleforc/proxyauthK8s/commit/01c762c3f4af03e96d330217697539771b468538)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- merge docs/rfc-0012-implemented (#300) - ([6f89ed3](https://github.com/batleforc/proxyauthK8s/commit/6f89ed341a370e8e307cd082d935b98d611f12d7)) - Maxime
- merge feat/rfc-0012-gaps (#297) - ([a6c0c63](https://github.com/batleforc/proxyauthK8s/commit/a6c0c63fea492f15921bbf760673d717baa7daf8)) - Maxime, Claude Opus 5.5 (1M context)
- merge feat/rfc-0012-phase-4 (#296) - ([2e7523a](https://github.com/batleforc/proxyauthK8s/commit/2e7523aa2c2d38456cd580d6447f61ad0f70e036)) - Maxime, Claude Opus 5.5 (1M context)
- merge feat/rfc-0012-phase-2 (#295) - ([664cf59](https://github.com/batleforc/proxyauthK8s/commit/664cf595870fc4022bc3fe42122fd693b3903acb)) - Maxime, Claude Opus 5.5 (1M context)
- merge feat/rfc-0012-agent-tunnel (#294) - ([64b008d](https://github.com/batleforc/proxyauthK8s/commit/64b008d25643fcec88adef91c9f3d04fce1a8cae)) - Maxime, Claude Opus 5.5 (1M context)
- merge docs/plan-0012-before-0010 (#293) - ([d8f7341](https://github.com/batleforc/proxyauthK8s/commit/d8f7341006eaebfebe2e62cb3f21270e4f536dbe)) - Maxime
- merge feat/rfc-0011-kit-runtime (#292) - ([5875558](https://github.com/batleforc/proxyauthK8s/commit/5875558ee64278c3e5ed909ff82168e0b8be7203)) - Maxime
- merge release/v0.3.0 (#290) - ([3a6f8f9](https://github.com/batleforc/proxyauthK8s/commit/3a6f8f9265806f58860710b4f807447737c7d217)) - Maxime
- merge ci/integration-timeout-45 (#291) - ([16cbc22](https://github.com/batleforc/proxyauthK8s/commit/16cbc2252ce581d371ccd3636d7868b302982b57)) - Maxime

- - -

## [v0.3.0](https://github.com/batleforc/proxyauthK8s/compare/cf10d5b4930095c38a16c7a1046a80d10e4b2b20..v0.3.0) - 2026-10-22
#### Features
- (**audit**) scan the built server and front images with trivy - ([ea08247](https://github.com/batleforc/proxyauthK8s/commit/ea08247b45c552370f6f10d35b0c05cbaab15685)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**docs**) the CRD IDE on kit v0.24.0, clusters and their accesses drawn - ([0ac16f9](https://github.com/batleforc/proxyauthK8s/commit/0ac16f933718086862e19ab072bf16adbf8cec91)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Bug Fixes
- (**audit**) allow the kit git source, crdgen unpublished - ([cf417ff](https://github.com/batleforc/proxyauthK8s/commit/cf417ffb687aa9398529454c6a9d572bb45a3685)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Documentation
- install the kubectl plugin with mise, fix the manual install - ([897f7b7](https://github.com/batleforc/proxyauthK8s/commit/897f7b7fdfb2ff312a0b91f4b736e94ff921eefd)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- (**fuzz**) bump workspace crates to 0.2.1 in fuzz/Cargo.lock - ([2ac90af](https://github.com/batleforc/proxyauthK8s/commit/2ac90affccd90a5c6d5081f8798268dc62ab63fd)) - Max Batleforc, Claude Opus 5.5 (1M context)
- merge feat/kit-schematic-kinds (#289) - ([d91cead](https://github.com/batleforc/proxyauthK8s/commit/d91ceade3dbb2b5ecb6c78b75b91574ce550c8ea)) - Maxime
- merge release/v0.2.1 (#287) - ([cf10d5b](https://github.com/batleforc/proxyauthK8s/commit/cf10d5b4930095c38a16c7a1046a80d10e4b2b20)) - Maxime

- - -

## [v0.2.1](https://github.com/batleforc/proxyauthK8s/compare/d0a46d88fe503a79385344c0ac4d88ce6f006211..v0.2.1) - 2026-10-20
#### Bug Fixes
- (**deps**) rustls-webpki 0.103.16 and webpki-root-certs 1.0.10 - ([6519d2a](https://github.com/batleforc/proxyauthK8s/commit/6519d2a5d3e6bbead0721afddb2c4351df4a1a41)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Continuous Integration
- (**release**) sign the chart with registry credentials, render krew - ([33af5ba](https://github.com/batleforc/proxyauthK8s/commit/33af5ba5b8008805bc4fa19fbf00e75d3558acc1)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- merge fix/rustsec-2026-0382-0383 (#286) - ([943ee28](https://github.com/batleforc/proxyauthK8s/commit/943ee281c6f01b98a1c21766b7a7dfa43b5868b5)) - Maxime
- merge ci/release-chart-sign-krew (#284) - ([a4654b9](https://github.com/batleforc/proxyauthK8s/commit/a4654b98a68d0f32b9239374159dcccdf0fb9136)) - Maxime
- merge release/v0.2.0 (#283) - ([d0a46d8](https://github.com/batleforc/proxyauthK8s/commit/d0a46d88fe503a79385344c0ac4d88ce6f006211)) - Maxime

- - -

## [v0.2.0](https://github.com/batleforc/proxyauthK8s/compare/6af8b40b9390a29f1fcd73545ec4a5214bbef935..v0.2.0) - 2026-10-20
#### Features
- (**CLI**) Creat Kubeconfig if not exist and create a tmux conf to open - ([c87b0fe](https://github.com/batleforc/proxyauthK8s/commit/c87b0fedb476514d8a3ac6f856289ef3258747a7)) - Max Batleforc
- (**api**) upgrade tunnel on hyper::upgrade (RFC 0007 phase 5) - ([8d7891a](https://github.com/batleforc/proxyauthK8s/commit/8d7891a94110bc72160b9e586f5b71116699f489)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) server, middlewares and OpenAPI on axum (RFC 0007 phase 3) - ([2203c2a](https://github.com/batleforc/proxyauthK8s/commit/2203c2a9180530275e56c960c9219540bc00f317)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**audit**) what an exec runs, on its audit lines (RFC 0004 phase 1) - ([cf2126a](https://github.com/batleforc/proxyauthK8s/commit/cf2126a2dd1a1065fadf5c126ffa0b3417ed2953)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**audit**) session_closed at the end of every tunnel (RFC 0008 phase 4) - ([018ac45](https://github.com/batleforc/proxyauthK8s/commit/018ac45a74cd4cb94ea02cb0c26bad134f24d588)) - Max Batleforc, Claude Opus 5.5 (1M context)
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) (**audit**) versioned audit trail (RFC 0008 phases 1-3) - ([82d07c8](https://github.com/batleforc/proxyauthK8s/commit/82d07c8dc49d8f2b77641930b63a1899c01c4933)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**chart**) ingress.apiHost, a clusters-only host for oc login --web - ([e161e25](https://github.com/batleforc/proxyauthK8s/commit/e161e258b474adc376bf1b5449fdcd475e26acea)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**cli**) PROXYAUTH_KEYRING_SERVICE, one token store per profile - ([2f935a1](https://github.com/batleforc/proxyauthK8s/commit/2f935a1700ee94e6482ada89c565f2ef28bb7ef4)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**docs**) adopt @weebo/operator-docs (RFC 0005 phase 3) - ([f0004c3](https://github.com/batleforc/proxyauthK8s/commit/f0004c31d88d798aed8b491535b3d347b443c383)) - Max Batleforc, Claude Opus 5.5 (1M context)
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) (**security**) exec and attach policy (RFC 0004 phases 2-3) - ([bfd094b](https://github.com/batleforc/proxyauthK8s/commit/bfd094b822fd328e9e90c99a787218d218e2980c)) - Max Batleforc, Claude Opus 5.5 (1M context)
- integration bench and §6.P matrix (RFC 0009 phases 1-2) - ([e40d9cb](https://github.com/batleforc/proxyauthK8s/commit/e40d9cb09372bf89d4f54303e3cecd410f7c63a6)) - Max Batleforc, Claude Opus 5.5 (1M context)
- step 6 /7 /9 - ([5329ba3](https://github.com/batleforc/proxyauthK8s/commit/5329ba34bc3c27678d3ec0177d11b540b8b53c5a)) - Max Batleforc
- Step 1-5 - ([d99b783](https://github.com/batleforc/proxyauthK8s/commit/d99b7835c2cfd3ca2d56968720c2b01f2e7b0f19)) - Max Batleforc
- last review - ([d5995f7](https://github.com/batleforc/proxyauthK8s/commit/d5995f7ab5139a068363632a7ee1cfb4817a80ad)) - Max Batleforc
- review RFC - ([a581c67](https://github.com/batleforc/proxyauthK8s/commit/a581c67bf69cc6427d9830f9964162277db3fe62)) - Max Batleforc
- wip - ([243f339](https://github.com/batleforc/proxyauthK8s/commit/243f339b6c86a5961d988342085a19e8577f4a52)) - Max Batleforc
- gestion skills et correctif - ([03985d0](https://github.com/batleforc/proxyauthK8s/commit/03985d027372c9f1f450d90b770185fe18f49fa7)) - Max Batleforc
- Apply reco - ([0a5921a](https://github.com/batleforc/proxyauthK8s/commit/0a5921a8007ac9df6b0beabc661120485b29fc9e)) - Max Batleforc
- Upgrade batch - ([f3896e7](https://github.com/batleforc/proxyauthK8s/commit/f3896e7d9413bb1c9f71c136a793fdf79450429b)) - Max Batleforc
- mise en place trust cert, login with or without browser, why we - ([4f1d427](https://github.com/batleforc/proxyauthK8s/commit/4f1d427a8c23e8280172627216263666572ced74)) - Max Batleforc
- Work on making this code more secure (and it will take a lot of work) - ([f0c7667](https://github.com/batleforc/proxyauthK8s/commit/f0c7667e18a9f84b66bcd63db169f8bc079d022d)) - Max Batleforc
- work toward making it a little bit more prod ready - ([dc05ab0](https://github.com/batleforc/proxyauthK8s/commit/dc05ab03f8c14e4b40fc276a691db50967f497d3)) - Max Batleforc
- correct past bug - ([3a776b9](https://github.com/batleforc/proxyauthK8s/commit/3a776b924adc98118e1e86a3a3ab11c4eb7bb13c)) - Max Batleforc
- wip - ([b3191cf](https://github.com/batleforc/proxyauthK8s/commit/b3191cf66fed01813b078dea4ad8e713ff9058a4)) - Max Batleforc
- fix most things - ([52481b0](https://github.com/batleforc/proxyauthK8s/commit/52481b00028fe181d29359a67763a91d9731637b)) - Max Batleforc
- Apply template and start work on port forward - ([a5688d5](https://github.com/batleforc/proxyauthK8s/commit/a5688d577ea03497d29de6e28e07bfe9a78f8e73)) - Max Batleforc
- We are back baby - ([1005f2c](https://github.com/batleforc/proxyauthK8s/commit/1005f2c8bdf6440eeb8193ae192e1ccdc82fb441)) - Max Batleforc
- wip - ([7ab304e](https://github.com/batleforc/proxyauthK8s/commit/7ab304e106d72646f2d425453ad4ff6699f2c3fa)) - Max Batleforc
- review and code coverage - ([0ad4ac9](https://github.com/batleforc/proxyauthK8s/commit/0ad4ac984921f10fc82d8aa812abd2a496e9be85)) - Max Batleforc
- Revue de code, validation dex, correction ci - ([580d331](https://github.com/batleforc/proxyauthK8s/commit/580d331fb05492e42dbaa59f66d0264c41b200cd)) - Max Batleforc
- wip - ([13aaab1](https://github.com/batleforc/proxyauthK8s/commit/13aaab1ff2047197a0800d580b12c8657e5b9896)) - Max Batleforc
- wip - ([4463fc1](https://github.com/batleforc/proxyauthK8s/commit/4463fc1f82ae611783acba63836d97ee5f422de4)) - Max Batleforc
- wip - ([4cba221](https://github.com/batleforc/proxyauthK8s/commit/4cba2210f0b4ed835e9e31b8272d0a649547fcde)) - Max Batleforc
- Code review and get back on track (#228) - ([97b0848](https://github.com/batleforc/proxyauthK8s/commit/97b0848f719a0e579644660c74f1b4d7de1ae844)) - Maxime
- change uri - ([152b14d](https://github.com/batleforc/proxyauthK8s/commit/152b14df945de163c201ffc3de860daccb718586)) - Maxime
- We are back baby - ([b1f6391](https://github.com/batleforc/proxyauthK8s/commit/b1f63912cf6cc223bc85d3a5ca291a0967cdeba0)) - Max Batleforc
- Work on validating token against oidc provider - ([fc76793](https://github.com/batleforc/proxyauthK8s/commit/fc767931034aee2abd4a3966d149d00da93909c1)) - Max Batleforc
- Mise en place d'une page formulaire assistant a la création de - ([d7973cd](https://github.com/batleforc/proxyauthK8s/commit/d7973cd54a6819df212e11bb2b2afdd529cf4fc2)) - Max Batleforc
- wip why k9s does not work ? - ([f55aae1](https://github.com/batleforc/proxyauthK8s/commit/f55aae19fce914675d95d374a504185c404afbba)) - Max Batleforc
- Add a scripts to check root level cargo.toml dep usage - ([750ae94](https://github.com/batleforc/proxyauthK8s/commit/750ae9414f2f1dcc0fdb20ebb8d9171aa0322034)) - Max Batleforc
- Trim rust dependency - ([f9a8799](https://github.com/batleforc/proxyauthK8s/commit/f9a87990229b754c370d8ecdfdc9e775b899583a)) - Max Batleforc
- Handle Async like Websocket or SPYDF and facto tls config - ([2e8f15a](https://github.com/batleforc/proxyauthK8s/commit/2e8f15abe3a7268411e43026b2a832a2c5083be5)) - Max Batleforc
- Upgrade package and start work on get_token - ([73b25fd](https://github.com/batleforc/proxyauthK8s/commit/73b25fde9b5ff59359fdfa26ba3334c6ed818896)) - Max Batleforc
- impl Get and work on login - ([c66f92e](https://github.com/batleforc/proxyauthK8s/commit/c66f92ef125903c664c951a2235376546cb18cce)) - Max Batleforc
- add docs on some actions workflow in order to have a clearer view - ([daf2d0a](https://github.com/batleforc/proxyauthK8s/commit/daf2d0a4104ea9445fa623c8cac2d3383cb6d731)) - Max Batleforc
- upgrade ci/cd - ([63d2f3a](https://github.com/batleforc/proxyauthK8s/commit/63d2f3a0a7fe900e8a320858d06c60edeae4f910)) - Max Batleforc
- Upgrade package - ([3d35327](https://github.com/batleforc/proxyauthK8s/commit/3d35327ab163e4d5178a5c0124dd9aeea7a1eba2)) - Max Batleforc
- Setup new Action to build/scan the image (and split the trivy job - ([6d695fa](https://github.com/batleforc/proxyauthK8s/commit/6d695fa1bbe824b2abdedd1e0622f794df8a4e90)) - Max Batleforc
- Setup helm chart linting - ([2c0ba14](https://github.com/batleforc/proxyauthK8s/commit/2c0ba143dc309211a6594cd1610d926f2df5ecc0)) - Max Batleforc
- Upgrade Reqwest - ([e9f8ba5](https://github.com/batleforc/proxyauthK8s/commit/e9f8ba5518d32b1ef0eca31250b47ab7e0129611)) - Max Batleforc
- add crd docs page - ([9519ec0](https://github.com/batleforc/proxyauthK8s/commit/9519ec0ffc3f25b5140d4891491266911a9774b3)) - Max Batleforc
- upgrade package - ([e2fd966](https://github.com/batleforc/proxyauthK8s/commit/e2fd966df0eff216d91002b0712271171334abe1)) - Max Batleforc
- [StepSecurity] Apply security best practices (#118) - ([d4631e3](https://github.com/batleforc/proxyauthK8s/commit/d4631e36bb7b4b365ac2c8a596a5d45ddca7cbce)) - StepSecurity Bot
- Mise en place de la doc (#101) - ([6af8b40](https://github.com/batleforc/proxyauthK8s/commit/6af8b40b9390a29f1fcd73545ec4a5214bbef935)) - Maxime
#### Bug Fixes
- (**api**) list fallback asks the apiserver's own access question - ([bcfd3f5](https://github.com/batleforc/proxyauthK8s/commit/bcfd3f54cfa8c07e9478fa1f11760979a7d4e276)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) virtual APIs in aggregated discovery - ([5c28e4d](https://github.com/batleforc/proxyauthK8s/commit/5c28e4dda82b5acd90f12c8d8bb69cb0dcc0367c)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**front**) match the theme script's end tag with attributes too - ([d036ad7](https://github.com/batleforc/proxyauthK8s/commit/d036ad76be27398d3a9d696b16345f9a9668a1e4)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**front**) find the inline theme script whatever the tag's case - ([6c8347c](https://github.com/batleforc/proxyauthK8s/commit/6c8347cfff65e3b8683b38b6fe420a12dded7d1f)) - Max Batleforc, Claude Opus 5.5 (1M context)
- CodeQL alerts on test messages, envtest TLS and the CLI configuration - ([7140721](https://github.com/batleforc/proxyauthK8s/commit/714072110b70cc84b472c8fcab15a774e8a7f9a1)) - Max Batleforc, Claude Opus 5.5 (1M context)
- What's left in the TODO List - ([4af1902](https://github.com/batleforc/proxyauthK8s/commit/4af190273051e0dfc09eb11e343dfff26a5c7d64)) - Max Batleforc
- wrong url - ([6c0978b](https://github.com/batleforc/proxyauthK8s/commit/6c0978bad5f69653069d4ae6fc6406cbad5fe38e)) - Max Batleforc
- remove cpu - ([a71711d](https://github.com/batleforc/proxyauthK8s/commit/a71711dddaaced340f9bf851123353ca12cd90eb)) - Maxime
- Validating against kube api, passing the bearer token need to be - ([639ba17](https://github.com/batleforc/proxyauthK8s/commit/639ba172c295b9b4143e8f71099b5fbf1dae7724)) - Max Batleforc
- Watch and follow not working (Idem for K9S) also add traces in - ([1313df0](https://github.com/batleforc/proxyauthK8s/commit/1313df0a0c23b3f8e97a2788b268ba9d42db0963)) - Max Batleforc
- Know if your are the leader before trying to reconcile and WIP on - ([0f6d6a3](https://github.com/batleforc/proxyauthK8s/commit/0f6d6a31468410b5b0cd59202fd1ba69d90c73d2)) - Max Batleforc
- upgrade tsconfig - ([c1901d6](https://github.com/batleforc/proxyauthK8s/commit/c1901d6a30730bbaf5b4ec998d8b36c54c687f31)) - Max Batleforc
- Ring injection not recognized since upgrade - ([828be21](https://github.com/batleforc/proxyauthK8s/commit/828be21249c84adb019034288dd584919caeba6b)) - Max Batleforc
- upgrade front - ([dc206fd](https://github.com/batleforc/proxyauthK8s/commit/dc206fd4ab9915655bc33e2e60682a7355927537)) - Max Batleforc
- change path in the readme to public path - ([b095552](https://github.com/batleforc/proxyauthK8s/commit/b0955520adc4373d93632153bd45dff43f872e23)) - Max Batleforc
- revert output change and make the doc build - ([d011fe3](https://github.com/batleforc/proxyauthK8s/commit/d011fe3943069ea31df7a3a03bbbc849f3d1f18c)) - Max Batleforc
#### Documentation
- (**rfc**) RFC 0007 ships in 0.2.0, as PLAN.md says - ([883155e](https://github.com/batleforc/proxyauthK8s/commit/883155e6aedd28ee76536b65f4ec4adce7de6ad3)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0011, conditions, Events and reason codes from the kit - ([5ec0a75](https://github.com/batleforc/proxyauthK8s/commit/5ec0a75cdc1895eb0f37f951737e98b6218f6b8a)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) plan, what is left before 0.2.0 - ([a185536](https://github.com/batleforc/proxyauthK8s/commit/a185536ca60895ab2312906a3b8ab1d854b35792)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0005 phase 3 waits for the kit's nested CRD reference - ([6709112](https://github.com/batleforc/proxyauthK8s/commit/67091126c4195a57b70cc490309d41b745c24e42)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) correct why oc login fails in RFC 0008 §17 - ([4cabc9c](https://github.com/batleforc/proxyauthK8s/commit/4cabc9c00918937954e4073ab4588a009bdd26e5)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0004 accepted after its third grill - ([d49847c](https://github.com/batleforc/proxyauthK8s/commit/d49847cd1dff952854ed3b69164d261ea379265f)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0010 accepted after its grill - ([7f9becd](https://github.com/batleforc/proxyauthK8s/commit/7f9becd9924c6c7f6e85b253d05e2cd9b504938a)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0008 accepted after its grill - ([2da7414](https://github.com/batleforc/proxyauthK8s/commit/2da7414dde861268fde43011617d8ed8ba123480)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**rfc**) RFC 0010, token exchange (draft) - ([9a46301](https://github.com/batleforc/proxyauthK8s/commit/9a463018d3225ae515d17e75c8a5675feede8f7b)) - Max Batleforc, Claude Opus 5.5 (1M context)
- land RFC 0001, 0002, 0003, 0006 and 0009, measure RFC 0007 point 9 - ([3be7140](https://github.com/batleforc/proxyauthK8s/commit/3be7140dd1e919b6b1cdd0fce591c23d366c1f61)) - Max Batleforc, Claude Opus 5.5 (1M context)
- developer guide for kit tasks, RFC 0005 implemented (phase 5) - ([2f87742](https://github.com/batleforc/proxyauthK8s/commit/2f87742e2ac10130fd1acedccd0de6c5b48628ee)) - Max Batleforc, Claude Opus 5.5 (1M context)
- the mediated OAuth server is not oc login compatible yet - ([da7e011](https://github.com/batleforc/proxyauthK8s/commit/da7e0119352e6925373af4bcebd5a5650c726e6f)) - Max Batleforc, Claude Opus 5.5 (1M context)
- close RFC 0007 phases 2-6 - ([19d317a](https://github.com/batleforc/proxyauthK8s/commit/19d317a97e983326956f0d1630d0bf5b11da3738)) - Max Batleforc, Claude Opus 5.5 (1M context)
- integration suite guide and release gate (RFC 0009 phase 4) - ([77c9ece](https://github.com/batleforc/proxyauthK8s/commit/77c9ecedfb982f22370d6e6f1b1c446b194927d1)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Tests
- (**api**) wait for the OIDC discovery retries instead of a 300 ms window - ([54a1c26](https://github.com/batleforc/proxyauthK8s/commit/54a1c26b460290bc2011bb6f9263cd6764e62259)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) no user data in the envtest group assertion message - ([c988d24](https://github.com/batleforc/proxyauthK8s/commit/c988d247d82a4fec5603bd668a435eae7e359386)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) retry an apply that lost a creation race - ([26cd260](https://github.com/batleforc/proxyauthK8s/commit/26cd26096cb1ccaa797dbd741290418b9b88085b)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) read only kubectl's errors in the attach check - ([d79a0f3](https://github.com/batleforc/proxyauthK8s/commit/d79a0f364795c93fb729b6545a9a2d7c80523241)) - Max Batleforc, Claude Opus 5.5 (1M context)
- integration tests for RFC 0002, 0003 and 0006 (RFC 0009 phase 3) - ([f284a94](https://github.com/batleforc/proxyauthK8s/commit/f284a94f31ead36e8533ca3c7252b8ade70b87c4)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Build system
- (**deps**) k8s.io/apiserver 0.37.1 for the request info reference - ([07d2c8f](https://github.com/batleforc/proxyauthK8s/commit/07d2c8f53fdf1978b0b087b94a0be0129ba841ea)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**deps**) bump github/codeql-action from 4.34.1 to 4.35.0 (#157) - ([8492c7d](https://github.com/batleforc/proxyauthK8s/commit/8492c7d63278e9ef5f7cad5ad767b49fc06f5e5c)) - dependabot[bot], dependabot[bot]
- (**deps**) bump actions/configure-pages from 5.0.0 to 6.0.0 (#153) - ([33ceac7](https://github.com/batleforc/proxyauthK8s/commit/33ceac704c8510932ffbee5da8e5efb1920ded08)) - dependabot[bot], dependabot[bot]
- (**deps**) bump actions/deploy-pages from 4.0.5 to 5.0.0 (#154) - ([038d043](https://github.com/batleforc/proxyauthK8s/commit/038d0434593fb75294c03b38711f8f1c419d956b)) - dependabot[bot], dependabot[bot]
- (**deps**) bump go-task/setup-task from 1.1.0 to 2.0.0 (#132) - ([ccd4d2e](https://github.com/batleforc/proxyauthK8s/commit/ccd4d2efa965fdd369c78f502e71b2101779ee58)) - dependabot[bot], dependabot[bot]
- (**deps**) bump github/codeql-action from 4.33.0 to 4.34.0 (#133) - ([c932746](https://github.com/batleforc/proxyauthK8s/commit/c93274661441d8914b5ef8f1b0b997262603652a)) - dependabot[bot], dependabot[bot]
- (**deps**) bump actions/upload-pages-artifact from 3.0.1 to 4.0.0 (#121) - ([f1c0515](https://github.com/batleforc/proxyauthK8s/commit/f1c051596e7b7f916a6eff90f0648ae47de30678)) - dependabot[bot], dependabot[bot]
- (**deps**) bump go-task/setup-task from 1.0.0 to 1.1.0 (#120) - ([ffb88fa](https://github.com/batleforc/proxyauthK8s/commit/ffb88fae42e36f3853222d4a05f36ebc7f4bb267)) - dependabot[bot], dependabot[bot]
- (**deps**) bump actions/configure-pages from 4.0.0 to 5.0.0 (#119) - ([0297a17](https://github.com/batleforc/proxyauthK8s/commit/0297a177087916e39a42d5b03592566cfdfe9ce7)) - dependabot[bot], dependabot[bot]
- (**deps**) bump bsord/helm-push from 4.1.0 to 4.2.0 (#102) - ([6aca380](https://github.com/batleforc/proxyauthK8s/commit/6aca38080971ed5a27e95d04abb0dfb3ba05b6d8)) - dependabot[bot], dependabot[bot]
- (**deps**) bump github/codeql-action from 4.32.6 to 4.33.0 (#115) - ([1f2307c](https://github.com/batleforc/proxyauthK8s/commit/1f2307c66be03e5ef0513984acd766f4bbb0ad2f)) - dependabot[bot], dependabot[bot]
- (**deps**) bump softprops/action-gh-release from 2.5.0 to 2.6.1 (#114) - ([c2b12c9](https://github.com/batleforc/proxyauthK8s/commit/c2b12c9a912387b52724e86cabcd2aff399c60c7)) - dependabot[bot], dependabot[bot]
- (**deps**) bump anchore/sbom-action from 0.23.0 to 0.23.1 (#109) - ([85d5ce4](https://github.com/batleforc/proxyauthK8s/commit/85d5ce4643f3f9bc0dbf9b66a10df11443541741)) - dependabot[bot], dependabot[bot]
- (**docs**) move .docs to docs - ([6a0d46b](https://github.com/batleforc/proxyauthK8s/commit/6a0d46bf5f8b2b4bf5fb6035ede57bfdf99577d0)) - Max Batleforc, Claude Opus 5.5 (1M context)
- kit:sync on weebo-operator-kit v0.20.0 (RFC 0005 phase 4) - ([8f28433](https://github.com/batleforc/proxyauthK8s/commit/8f2843392ffc411470190892519771e2448b7ca4)) - Max Batleforc, Claude Opus 5.5 (1M context)
- ESLint 10 for the docs, tsconfigs ready for TypeScript 7 - ([bc81637](https://github.com/batleforc/proxyauthK8s/commit/bc81637fdb9658205770606b1770ab57a1d2d3b9)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Continuous Integration
- (**integration**) raise the keyring quota before the suite - ([3cea1c9](https://github.com/batleforc/proxyauthK8s/commit/3cea1c9b25e98d5c483036641655f728cee78ad2)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**integration**) install cargo-binstall before cargo-nextest - ([9ae092c](https://github.com/batleforc/proxyauthK8s/commit/9ae092c028a0420143e89820afe26ba78395b48e)) - Max Batleforc, Claude Opus 5.5 (1M context)
- skip the integration suite from a job, not a twin workflow - ([28170cc](https://github.com/batleforc/proxyauthK8s/commit/28170cc00a1e591f9a3a38d3cf0b5b6124399062)) - Max Batleforc, Claude Opus 5.5 (1M context)
- install pnpm through mise, not pnpm/action-setup - ([3bc39ff](https://github.com/batleforc/proxyauthK8s/commit/3bc39ffd8de0434a8e9529287bac475dfd48ee51)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Refactoring
- (**api**) standard proxy and watch on axum bodies (RFC 0007 phase 4) - ([a3fe1a6](https://github.com/batleforc/proxyauthK8s/commit/a3fe1a6fe24b01eba867dc92d131b77dd7491bba)) - Max Batleforc, Claude Opus 5.5 (1M context)
- (**api**) test helpers and extractors on axum (RFC 0007 phase 2) - ([86ddbd2](https://github.com/batleforc/proxyauthK8s/commit/86ddbd2868c4c9f0ba3a9caf60dcd0514527a27b)) - Max Batleforc, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- (**version**) v0.1.9 - ([1e06530](https://github.com/batleforc/proxyauthK8s/commit/1e065305fe3ba1006d35df15bbf5e2ebf27a2295)) - Max Batleforc
- merge feat/oc-login-api-host (#282) - ([c68e089](https://github.com/batleforc/proxyauthK8s/commit/c68e0896a9ec2206d1fa21822b98144842c4f1ee)) - Maxime
- merge docs/rfc-landing-0007-0009 (#281) - ([949577d](https://github.com/batleforc/proxyauthK8s/commit/949577de4a6b6368687b58a07ffd37e85a8469a4)) - Maxime
- merge build/kit-sync-v0.20.0 (#280) - ([df8b153](https://github.com/batleforc/proxyauthK8s/commit/df8b153db435cd070b9a12c1bb5ef0b4ec62a4b5)) - Maxime
- merge fix/health-oidc-retry-flake (#279) - ([dceb3f3](https://github.com/batleforc/proxyauthK8s/commit/dceb3f30ed194aea098fdc69745cc79174839a79)) - Maxime
- merge feat/review (#253) - ([808d8c4](https://github.com/batleforc/proxyauthK8s/commit/808d8c4c6805aea74269fd7dc1a89d781eda709c)) - Maxime
- merge origin/main into feat/review - ([0615242](https://github.com/batleforc/proxyauthK8s/commit/06152427e48ea3802de63edd1ed352f58644b7ba)) - Max Batleforc, Claude Opus 5.5
- upgrade - ([70c56e4](https://github.com/batleforc/proxyauthK8s/commit/70c56e41da17f8cd26ec9b7dd84940990607f1e1)) - Max Batleforc
#### Style
- (**front**) prettier on csp-theme-hash.mjs - ([9fa37e5](https://github.com/batleforc/proxyauthK8s/commit/9fa37e514c577006d324abe472dbc5e7753c89e0)) - Max Batleforc, Claude Opus 5.5 (1M context)

- - -

## [v0.1.9](https://github.com/batleforc/proxyauthK8s/compare/6af8b40b9390a29f1fcd73545ec4a5214bbef935..v0.1.9) - 2026-03-20
#### Features
- add crd docs page - ([9519ec0](https://github.com/batleforc/proxyauthK8s/commit/9519ec0ffc3f25b5140d4891491266911a9774b3)) - Max Batleforc
- upgrade package - ([e2fd966](https://github.com/batleforc/proxyauthK8s/commit/e2fd966df0eff216d91002b0712271171334abe1)) - Max Batleforc
- [StepSecurity] Apply security best practices (#118) - ([d4631e3](https://github.com/batleforc/proxyauthK8s/commit/d4631e36bb7b4b365ac2c8a596a5d45ddca7cbce)) - StepSecurity Bot
- Mise en place de la doc (#101) - ([6af8b40](https://github.com/batleforc/proxyauthK8s/commit/6af8b40b9390a29f1fcd73545ec4a5214bbef935)) - Maxime
#### Bug Fixes
- upgrade front - ([dc206fd](https://github.com/batleforc/proxyauthK8s/commit/dc206fd4ab9915655bc33e2e60682a7355927537)) - Max Batleforc
- change path in the readme to public path - ([b095552](https://github.com/batleforc/proxyauthK8s/commit/b0955520adc4373d93632153bd45dff43f872e23)) - Max Batleforc
- revert output change and make the doc build - ([d011fe3](https://github.com/batleforc/proxyauthK8s/commit/d011fe3943069ea31df7a3a03bbbc849f3d1f18c)) - Max Batleforc
#### Build system
- (**deps**) bump actions/upload-pages-artifact from 3.0.1 to 4.0.0 (#121) - ([f1c0515](https://github.com/batleforc/proxyauthK8s/commit/f1c051596e7b7f916a6eff90f0648ae47de30678)) - dependabot[bot]
- (**deps**) bump go-task/setup-task from 1.0.0 to 1.1.0 (#120) - ([ffb88fa](https://github.com/batleforc/proxyauthK8s/commit/ffb88fae42e36f3853222d4a05f36ebc7f4bb267)) - dependabot[bot]
- (**deps**) bump actions/configure-pages from 4.0.0 to 5.0.0 (#119) - ([0297a17](https://github.com/batleforc/proxyauthK8s/commit/0297a177087916e39a42d5b03592566cfdfe9ce7)) - dependabot[bot]
- (**deps**) bump bsord/helm-push from 4.1.0 to 4.2.0 (#102) - ([6aca380](https://github.com/batleforc/proxyauthK8s/commit/6aca38080971ed5a27e95d04abb0dfb3ba05b6d8)) - dependabot[bot]
- (**deps**) bump github/codeql-action from 4.32.6 to 4.33.0 (#115) - ([1f2307c](https://github.com/batleforc/proxyauthK8s/commit/1f2307c66be03e5ef0513984acd766f4bbb0ad2f)) - dependabot[bot]
- (**deps**) bump softprops/action-gh-release from 2.5.0 to 2.6.1 (#114) - ([c2b12c9](https://github.com/batleforc/proxyauthK8s/commit/c2b12c9a912387b52724e86cabcd2aff399c60c7)) - dependabot[bot]
- (**deps**) bump anchore/sbom-action from 0.23.0 to 0.23.1 (#109) - ([85d5ce4](https://github.com/batleforc/proxyauthK8s/commit/85d5ce4643f3f9bc0dbf9b66a10df11443541741)) - dependabot[bot]

- - -

## [v0.1.8](https://github.com/batleforc/proxyauthK8s/compare/ba4825e3234c1266d39344b66a1f50a37973b329..v0.1.8) - 2026-03-07
#### Features
- pre bump commit - ([afa1033](https://github.com/batleforc/proxyauthK8s/commit/afa1033f1dd0bf8a3dfa8725393fa10a93a089f6)) - Max Batleforc
- Work on first real release - ([ba4825e](https://github.com/batleforc/proxyauthK8s/commit/ba4825e3234c1266d39344b66a1f50a37973b329)) - Max Batleforc

- - -

## [v0.1.7](https://github.com/batleforc/proxyauthK8s/compare/ef1e89362d07704161f0c67830e730a2c5be4e16..v0.1.7) - 2026-03-07
#### Features
- another other pre bump - ([76f9f0b](https://github.com/batleforc/proxyauthK8s/commit/76f9f0bfb7628e7e64ee533719dd83ca06fa6dce)) - Max Batleforc
- another pre-bump - ([6473b56](https://github.com/batleforc/proxyauthK8s/commit/6473b56382eb81ad35fcd25ff25688d1749bcf82)) - Max Batleforc
- pre-bump - ([35405ab](https://github.com/batleforc/proxyauthK8s/commit/35405ab917ed59655b413757290fee828bb5537b)) - Max Batleforc
- pre-bump - ([ef1e893](https://github.com/batleforc/proxyauthK8s/commit/ef1e89362d07704161f0c67830e730a2c5be4e16)) - Max Batleforc
#### Bug Fixes
- qip - ([8d41bd2](https://github.com/batleforc/proxyauthK8s/commit/8d41bd23efd673c93aaff65131a2970900448e7f)) - Max Batleforc
- wrong pre bump format - ([7a10ee2](https://github.com/batleforc/proxyauthK8s/commit/7a10ee295daff89f9505ee679a1cef7fc17fa992)) - Max Batleforc

- - -

## [v0.1.6](https://github.com/batleforc/proxyauthK8s/compare/bf92b5ce245a81ed33045155612402ce7020355f..v0.1.6) - 2026-03-07
### Packages
- [proxyauthk8s-back-crd-v0.1.0](libs/server/crd) locked to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crd-v0.1.0)
- [proxyauthk8s-back-trace-v0.1.0](libs/server/trace) locked to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-back-controller-v0.1.0](libs/server/controller) locked to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-controller-v0.1.0)
- [proxyauthk8s-back-kubectl-v0.1.0](apps/kubectl_proxyauth) locked to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-cli-trace-v0.1.0](libs/cli/cli_trace) locked to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-trace-v0.1.0)
- [proxyauthk8s-back-server-v0.1.0](apps/server) locked to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-back-common-v0.1.0](libs/server/common) locked to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-common-v0.1.0)
- [proxyauthk8s-front-api-v0.1.0](libs/front/front-api) locked to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-back-swaggergen-v0.1.0](apps/swaggergen) locked to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-swaggergen-v0.1.0)
- [proxyauthk8s-cli-v0.1.0](libs/cli/cli) locked to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-v0.1.0)
- [v0.0.0](deploy/chart) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-cli-client-api-v0.1.0](libs/cli/client_api) locked to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-client-api-v0.1.0)
- [v0.0.0](deploy/chart-crd) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-back-api-v0.1.0](libs/server/api) locked to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-api-v0.1.0)
- [proxyauthk8s-back-crdgen-v0.1.0](apps/crdgen) locked to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crdgen-v0.1.0)
- [proxyauthk8s-front-v0.1.0](apps/front) locked to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-v0.1.0)
### Global changes
#### Features
- wip - ([bf92b5c](https://github.com/batleforc/proxyauthK8s/commit/bf92b5ce245a81ed33045155612402ce7020355f)) - Max Batleforc

- - -

## [v0.1.5](https://github.com/batleforc/proxyauthK8s/compare/de7aeb447f771965d632c13c8824b0bf182c7879..v0.1.5) - 2026-03-06
### Packages
- [v0.0.0](deploy/chart) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-cli-trace-v0.1.0](libs/cli/cli_trace) locked to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-trace-v0.1.0)
- [proxyauthk8s-back-common-v0.1.0](libs/server/common) locked to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-common-v0.1.0)
- [proxyauthk8s-back-trace-v0.1.0](libs/server/trace) locked to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-cli-v0.1.0](libs/cli/cli) locked to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-v0.1.0)
- [proxyauthk8s-back-server-v0.1.0](apps/server) locked to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-cli-client-api-v0.1.0](libs/cli/client_api) locked to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-client-api-v0.1.0)
- [proxyauthk8s-back-kubectl-v0.1.0](apps/kubectl_proxyauth) locked to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-back-controller-v0.1.0](libs/server/controller) locked to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-controller-v0.1.0)
- [v0.0.0](deploy/chart-crd) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-front-api-v0.1.0](libs/front/front-api) locked to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-back-crdgen-v0.1.0](apps/crdgen) locked to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crdgen-v0.1.0)
- [proxyauthk8s-front-v0.1.0](apps/front) locked to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-v0.1.0)
- [proxyauthk8s-back-api-v0.1.0](libs/server/api) locked to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-api-v0.1.0)
- [proxyauthk8s-back-swaggergen-v0.1.0](apps/swaggergen) locked to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-swaggergen-v0.1.0)
- [proxyauthk8s-back-crd-v0.1.0](libs/server/crd) locked to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crd-v0.1.0)
### Global changes
#### Features
- wip - ([de7aeb4](https://github.com/batleforc/proxyauthK8s/commit/de7aeb447f771965d632c13c8824b0bf182c7879)) - Max Batleforc

- - -

## [v0.1.4](https://github.com/batleforc/proxyauthK8s/compare/bf2459868ff191790662005a41c5078d8141dda4..v0.1.4) - 2026-03-06
### Packages
- [proxyauthk8s-back-controller-v0.1.0](libs/server/controller) locked to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-controller-v0.1.0)
- [proxyauthk8s-cli-v0.1.0](libs/cli/cli) locked to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-v0.1.0)
- [proxyauthk8s-back-api-v0.1.0](libs/server/api) locked to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-api-v0.1.0)
- [proxyauthk8s-back-swaggergen-v0.1.0](apps/swaggergen) locked to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-swaggergen-v0.1.0)
- [proxyauthk8s-front-api-v0.1.0](libs/front/front-api) locked to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-back-crd-v0.1.0](libs/server/crd) locked to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crd-v0.1.0)
- [proxyauthk8s-back-server-v0.1.0](apps/server) locked to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-cli-trace-v0.1.0](libs/cli/cli_trace) locked to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-trace-v0.1.0)
- [proxyauthk8s-back-crdgen-v0.1.0](apps/crdgen) locked to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crdgen-v0.1.0)
- [proxyauthk8s-back-trace-v0.1.0](libs/server/trace) locked to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-cli-client-api-v0.1.0](libs/cli/client_api) locked to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-client-api-v0.1.0)
- [v0.0.0](deploy/chart-crd) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-back-common-v0.1.0](libs/server/common) locked to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-common-v0.1.0)
- [proxyauthk8s-back-kubectl-v0.1.0](apps/kubectl_proxyauth) locked to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-front-v0.1.0](apps/front) locked to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-v0.1.0)
- [v0.0.0](deploy/chart) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
### Global changes
#### Features
- pre-bump - ([bf24598](https://github.com/batleforc/proxyauthK8s/commit/bf2459868ff191790662005a41c5078d8141dda4)) - Max Batleforc

- - -

## [v0.1.3](https://github.com/batleforc/proxyauthK8s/compare/dd162f80c0df38be36dd8076b3ac1a5139a4ecb8..v0.1.3) - 2026-03-06
### Packages
- [proxyauthk8s-cli-client-api-v0.1.0](libs/cli/client_api) locked to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-client-api-v0.1.0)
- [proxyauthk8s-front-v0.1.0](apps/front) locked to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-v0.1.0)
- [proxyauthk8s-back-controller-v0.1.0](libs/server/controller) locked to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-controller-v0.1.0)
- [v0.0.0](deploy/chart-crd) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-back-api-v0.1.0](libs/server/api) locked to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-api-v0.1.0)
- [v0.0.0](deploy/chart) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-cli-trace-v0.1.0](libs/cli/cli_trace) locked to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-trace-v0.1.0)
- [proxyauthk8s-front-api-v0.1.0](libs/front/front-api) locked to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-back-server-v0.1.0](apps/server) locked to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-back-crd-v0.1.0](libs/server/crd) locked to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crd-v0.1.0)
- [proxyauthk8s-back-trace-v0.1.0](libs/server/trace) locked to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-back-swaggergen-v0.1.0](apps/swaggergen) locked to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-swaggergen-v0.1.0)
- [proxyauthk8s-cli-v0.1.0](libs/cli/cli) locked to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-v0.1.0)
- [proxyauthk8s-back-kubectl-v0.1.0](apps/kubectl_proxyauth) locked to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-back-common-v0.1.0](libs/server/common) locked to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-common-v0.1.0)
- [proxyauthk8s-back-crdgen-v0.1.0](apps/crdgen) locked to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crdgen-v0.1.0)
### Global changes
#### Features
- pre-bump - ([73f7d44](https://github.com/batleforc/proxyauthK8s/commit/73f7d44ddc2984c31588147fd2a1d3966da84e06)) - Max Batleforc
- update containerfile - ([dd162f8](https://github.com/batleforc/proxyauthK8s/commit/dd162f80c0df38be36dd8076b3ac1a5139a4ecb8)) - Max Batleforc

- - -

## [v0.1.2](https://github.com/batleforc/proxyauthK8s/compare/c4853097d198ff6a8ec8033bd84de8f216ad48e9..v0.1.2) - 2026-03-06
### Packages
- [proxyauthk8s-back-trace-v0.1.0](libs/server/trace) locked to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-front-api-v0.1.0](libs/front/front-api) locked to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-cli-client-api-v0.1.0](libs/cli/client_api) locked to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-client-api-v0.1.0)
- [proxyauthk8s-cli-v0.1.0](libs/cli/cli) locked to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-v0.1.0)
- [proxyauthk8s-front-v0.1.0](apps/front) locked to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-v0.1.0)
- [proxyauthk8s-back-swaggergen-v0.1.0](apps/swaggergen) locked to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-swaggergen-v0.1.0)
- [proxyauthk8s-back-crd-v0.1.0](libs/server/crd) locked to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crd-v0.1.0)
- [proxyauthk8s-back-kubectl-v0.1.0](apps/kubectl_proxyauth) locked to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-back-controller-v0.1.0](libs/server/controller) locked to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-controller-v0.1.0)
- [proxyauthk8s-back-api-v0.1.0](libs/server/api) locked to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-api-v0.1.0)
- [proxyauthk8s-back-crdgen-v0.1.0](apps/crdgen) locked to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crdgen-v0.1.0)
- [proxyauthk8s-back-common-v0.1.0](libs/server/common) locked to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-common-v0.1.0)
- [v0.0.0](deploy/chart-crd) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-back-server-v0.1.0](apps/server) locked to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-cli-trace-v0.1.0](libs/cli/cli_trace) locked to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-trace-v0.1.0)
- [v0.0.0](deploy/chart) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
### Global changes
#### Features
- setup cog.toml to bump properly all package - ([c485309](https://github.com/batleforc/proxyauthK8s/commit/c4853097d198ff6a8ec8033bd84de8f216ad48e9)) - Max Batleforc

- - -

## [v0.1.1](https://github.com/batleforc/proxyauthK8s/compare/7e95d1599fa64f83e07bd315e4e3c13342dfb6af..v0.1.1) - 2026-03-06
### Packages
- [proxyauthk8s-back-trace-v0.1.0](libs/server/trace) locked to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-cli-trace-v0.1.0](libs/cli/cli_trace) locked to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-trace-v0.1.0)
- [proxyauthk8s-back-swaggergen-v0.1.0](apps/swaggergen) locked to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-swaggergen-v0.1.0)
- [v0.0.0](deploy/chart-crd) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
- [proxyauthk8s-back-api-v0.1.0](libs/server/api) locked to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-api-v0.1.0)
- [proxyauthk8s-front-v0.1.0](apps/front) locked to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-v0.1.0)
- [proxyauthk8s-front-api-v0.1.0](libs/front/front-api) locked to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-cli-v0.1.0](libs/cli/cli) locked to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-v0.1.0)
- [proxyauthk8s-back-kubectl-v0.1.0](apps/kubectl_proxyauth) locked to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-back-common-v0.1.0](libs/server/common) locked to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-common-v0.1.0)
- [proxyauthk8s-back-crd-v0.1.0](libs/server/crd) locked to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crd-v0.1.0)
- [proxyauthk8s-back-controller-v0.1.0](libs/server/controller) locked to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-controller-v0.1.0)
- [proxyauthk8s-back-crdgen-v0.1.0](apps/crdgen) locked to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-crdgen-v0.1.0)
- [proxyauthk8s-back-server-v0.1.0](apps/server) locked to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-cli-client-api-v0.1.0](libs/cli/client_api) locked to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/tree/proxyauthk8s-cli-client-api-v0.1.0)
- [v0.0.0](deploy/chart) locked to [v0.0.0](https://github.com/batleforc/proxyauthK8s/tree/v0.0.0)
### Global changes
#### Features
- commit post - ([7e95d15](https://github.com/batleforc/proxyauthK8s/commit/7e95d1599fa64f83e07bd315e4e3c13342dfb6af)) - Max Batleforc
#### Bug Fixes
- build and patch ! - ([5a954fd](https://github.com/batleforc/proxyauthK8s/commit/5a954fdb3671d378209087de0a188fbe049197fe)) - Max Batleforc

- - -

## [v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..v0.1.0) - 2026-03-06
### Package updates
- [proxyauthk8s-back-api](libs/server/api) bumped to [proxyauthk8s-back-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-api-v0.1.0)
- [proxyauthk8s-back-trace](libs/server/trace) bumped to [proxyauthk8s-back-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-trace-v0.1.0)
- [proxyauthk8s-cli-trace](libs/cli/cli_trace) bumped to [proxyauthk8s-cli-trace-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-cli-trace-v0.1.0)
- [proxyauthk8s-back-common](libs/server/common) bumped to [proxyauthk8s-back-common-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-common-v0.1.0)
- [proxyauthk8s-back-swaggergen](apps/swaggergen) bumped to [proxyauthk8s-back-swaggergen-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-swaggergen-v0.1.0)
- [proxyauthk8s-back-kubectl](apps/kubectl_proxyauth) bumped to [proxyauthk8s-back-kubectl-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-kubectl-v0.1.0)
- [proxyauthk8s-back-controller](libs/server/controller) bumped to [proxyauthk8s-back-controller-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-controller-v0.1.0)
- [proxyauthk8s-cli-client-api](libs/cli/client_api) bumped to [proxyauthk8s-cli-client-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-cli-client-api-v0.1.0)
- [proxyauthk8s-back-server](apps/server) bumped to [proxyauthk8s-back-server-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-server-v0.1.0)
- [proxyauthk8s-back-crd](libs/server/crd) bumped to [proxyauthk8s-back-crd-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-crd-v0.1.0)
- [proxyauthk8s-back-crdgen](apps/crdgen) bumped to [proxyauthk8s-back-crdgen-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-back-crdgen-v0.1.0)
- [proxyauthk8s-front-api](libs/front/front-api) bumped to [proxyauthk8s-front-api-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-front-api-v0.1.0)
- [proxyauthk8s-front](apps/front) bumped to [proxyauthk8s-front-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-front-v0.1.0)
- [proxyauthk8s-cli](libs/cli/cli) bumped to [proxyauthk8s-cli-v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/7e79a7cc31943ac20adc0a58a6b20659c83f1efa..proxyauthk8s-cli-v0.1.0)
### Global changes
#### Features
- (**CI/CD**) add Spectral Analysis and work on on tag release-prepare - ([483bd5f](https://github.com/batleforc/proxyauthK8s/commit/483bd5fb7eb5c426793e5526659d56bb3439ea4c)) - Max Batleforc
- (**LICENSE**) add LICENSE file - ([bb6adad](https://github.com/batleforc/proxyauthK8s/commit/bb6adad481f1a6f7c8600d1d76ae79aef0373095)) - Max Batleforc
- (**SECURITY**) add info on how to create a vuln report - ([ff424b5](https://github.com/batleforc/proxyauthK8s/commit/ff424b54ad2cd60d743268a33629f409887ca358)) - Max Batleforc
- (**SECURITY**) Add SECURITY file - ([35014b3](https://github.com/batleforc/proxyauthK8s/commit/35014b348bb6ed04627135992291cfcd55601877)) - Max Batleforc
- (**StepSecurity**) Harden GitHub Actions - ([5a2473f](https://github.com/batleforc/proxyauthK8s/commit/5a2473f1b6ebfeb7a9750093932cc6a070e4d949)) - Maxime
- (**StepSecurity**) Harden GitHub Actions - ([5fc5e23](https://github.com/batleforc/proxyauthK8s/commit/5fc5e2391f00671002c5a197d4101a75b3d87bbf)) - StepSecurity Bot
- (**api**) Generate rust api binding - ([db95357](https://github.com/batleforc/proxyauthK8s/commit/db953578bedc48c10aa92c66b0d7010e1f7bd1dd)) - Max Batleforc
- (**api**) Cluster delegated auth is working - ([024914c](https://github.com/batleforc/proxyauthK8s/commit/024914cfc8c797bdcfd5defcb2a355dc998f557a)) - Max Batleforc
- (**api**) WIP on the login endpoint, redirect is the next step - ([feebe7f](https://github.com/batleforc/proxyauthK8s/commit/feebe7f2fa29668a8201e05d8a828997068909d2)) - Max Batleforc
- (**api**) create crd to oidcconf - ([924b352](https://github.com/batleforc/proxyauthK8s/commit/924b3527b694f854bb333d22a1ef7de9fa8f4c48)) - Max Batleforc
- (**api**) Start work on delegated OIDC Handling - ([b800a59](https://github.com/batleforc/proxyauthK8s/commit/b800a5985e705dfc0e676ab6661b06f1d9fe6ba2)) - Max Batleforc
- (**api**) return the cluster visible by the current user ! - ([c0c72aa](https://github.com/batleforc/proxyauthK8s/commit/c0c72aa4671018c37709970c397437c36081c62c)) - Max Batleforc
- (**api**) Work on the auth validator for each route - ([ee570e8](https://github.com/batleforc/proxyauthK8s/commit/ee570e832baa4dc03b44b011ebe29666f414972c)) - Max Batleforc
- (**api**) WIP on auth - ([f866331](https://github.com/batleforc/proxyauthK8s/commit/f866331016ce5f90d402117c4dfa653e4b46ce1c)) - Max Batleforc
- (**api**) wip - ([de5b252](https://github.com/batleforc/proxyauthK8s/commit/de5b252a8f8efe90d668cee108cd16a98b7b8fe9)) - Max Batleforc
- (**auth**) add front authentication - ([c7b0ac1](https://github.com/batleforc/proxyauthK8s/commit/c7b0ac1bac82990ca0412e8e85c22f4ac75d6bff)) - Max Batleforc
- (**back**) upgrade package - ([aac6870](https://github.com/batleforc/proxyauthK8s/commit/aac6870f46a0d2f21caf0db771d83892213caed7)) - Max Batleforc
- (**chart**) add Otel - ([95d8599](https://github.com/batleforc/proxyauthK8s/commit/95d859970a20b982d6cc2575518a05da08e77872)) - Max Batleforc
- (**chart**) create crds chart - ([ede39d8](https://github.com/batleforc/proxyauthK8s/commit/ede39d85184420af05315ff82ebf55616c94b5f1)) - Max Batleforc
- (**charts**) add ressources to otel - ([474019b](https://github.com/batleforc/proxyauthK8s/commit/474019be476c3595f62d6714c591eb5b00be0cde)) - Max Batleforc
- (**ci**) add a step to build helm chart too - ([db787f3](https://github.com/batleforc/proxyauthK8s/commit/db787f37a56a3b2eb9e686f14f9d7cbe4cca4226)) - Max Batleforc
- (**ci/cd**) new ci to prepare a new release - ([806c019](https://github.com/batleforc/proxyauthK8s/commit/806c0191fdd2654af7cf891766b2ea6732c22a5b)) - Max Batleforc
- (**ci/cd**) add dispatch for check lint - ([b2b7bf1](https://github.com/batleforc/proxyauthK8s/commit/b2b7bf1203aabc5dd97f19439b37c366aae7f6b1)) - Max Batleforc
- (**ci/cd**) fix clippy and setup pipeline - ([535d194](https://github.com/batleforc/proxyauthK8s/commit/535d1946010fb5e6a89877cae1a3f3399f0af195)) - Max Batleforc
- (**cicd**) Add Scorecard workflow for supply-chain security analysis - ([421be68](https://github.com/batleforc/proxyauthK8s/commit/421be68c72fc1d0f3de5cfd6b15c3e5d77f5983e)) - Maxime
- (**cli**) Add clear keyring in the clear cli - ([a2b2eb5](https://github.com/batleforc/proxyauthK8s/commit/a2b2eb5fcf7f73d5c0f01a59edeabb80a520fe21)) - Max Batleforc
- (**cli**) clippy - ([9735ad4](https://github.com/batleforc/proxyauthK8s/commit/9735ad4fe89c00fd9288549a0d1fd537de49ec98)) - Max Batleforc
- (**cli**) Half the dev flow working ! - ([bbab41b](https://github.com/batleforc/proxyauthK8s/commit/bbab41be887f3382b7b0376797660c94bbf2f84b)) - Max Batleforc
- (**cli**) make 1/4 of the login work - ([46d8a99](https://github.com/batleforc/proxyauthK8s/commit/46d8a99f36d986ab11f2288a0e35a330dd93893f)) - Max Batleforc
- (**cli**) describe what the cli should do - ([c805934](https://github.com/batleforc/proxyauthK8s/commit/c80593426568884c10a004b199c2ff5d47459924)) - Max Batleforc
- (**client_api**) rename crate and fix it - ([a5d53ad](https://github.com/batleforc/proxyauthK8s/commit/a5d53ad4f193a34629db089bde58937c9cfa80ce)) - Max Batleforc
- (**cluster**) take into account the network - ([d38755b](https://github.com/batleforc/proxyauthK8s/commit/d38755be15b27726f7f3e49e6a31fa39d6d511e2)) - Max Batleforc
- (**cluster**) add timeout - ([541273a](https://github.com/batleforc/proxyauthK8s/commit/541273ae1c5a5778807d711dbcadcc3e96ba7f1d)) - Max Batleforc
- (**controller**) make sure the service is reachable before exposing it - ([fc3a91f](https://github.com/batleforc/proxyauthK8s/commit/fc3a91fc46885da1bbf5d2d581d045d2fdec49dd)) - Max Batleforc
- (**controller**) add from/to json to ProxyKubeApi - ([c3f7f14](https://github.com/batleforc/proxyauthK8s/commit/c3f7f14a71175b8bd4ab7bf778b9b232c32b3065)) - Max Batleforc
- (**controller**) Start créating listener - ([46b7bec](https://github.com/batleforc/proxyauthK8s/commit/46b7bec0f6ab213a07b8c934e3e6814c972f7aee)) - Max Batleforc
- (**controller**) init controller - ([17957cb](https://github.com/batleforc/proxyauthK8s/commit/17957cba1e9556ba0ed3ff2aeb049b4b6bd1102e)) - Max Batleforc
- (**crd**) make the CRD easyer to use - ([4aa1bbe](https://github.com/batleforc/proxyauthK8s/commit/4aa1bbe06d5929c9152f73c79fb3f097136d484d)) - Max Batleforc
- (**crd**) add oidc block - ([bf53b78](https://github.com/batleforc/proxyauthK8s/commit/bf53b78d9cd47e9c177bffb8acc7be59978fc400)) - Max Batleforc
- (**devfile**) correct dbgate/devfile and change port to my favorite one - ([22bde75](https://github.com/batleforc/proxyauthK8s/commit/22bde75adc8cfc258a68d1ce9dc8d219b4c1d76e)) - Max Batleforc
- (**devfile**) Add a Devfile - ([6d00fbb](https://github.com/batleforc/proxyauthK8s/commit/6d00fbb3df87381c25874a0349aed703da72aad7)) - Max Batleforc
- (**front**) nx migration migrate and npm update - ([a82478b](https://github.com/batleforc/proxyauthK8s/commit/a82478bb38962ad18c244f052d94c8f97c4096be)) - Max Batleforc
- (**front**) add cliView - ([58a2660](https://github.com/batleforc/proxyauthK8s/commit/58a2660de5c119b4a7e33dae897f78e76467711f)) - Max Batleforc
- (**front**) make lint work - ([bf11331](https://github.com/batleforc/proxyauthK8s/commit/bf11331444769244a9cbd3e691c1601ef7ead854)) - Max Batleforc
- (**front**) turn to dark mode - ([4a69d21](https://github.com/batleforc/proxyauthK8s/commit/4a69d212874b72e751c45a795063876033ff4bc1)) - Max Batleforc
- (**front**) add annotation and WIP getallvisiblecluster - ([fb0d3c9](https://github.com/batleforc/proxyauthK8s/commit/fb0d3c97077ce8f439762f0b37704a5e9d40b619)) - Max Batleforc
- (**front**) WIP on the nav bar - ([94006f3](https://github.com/batleforc/proxyauthK8s/commit/94006f3002a2d57decddd03c4cbc7c2b4b70ee90)) - Max Batleforc
- (**front**) add maz ui has a component provider - ([71a380a](https://github.com/batleforc/proxyauthK8s/commit/71a380aa3b6754faf4ab547125777af22454cdf4)) - Max Batleforc
- (**front**) add missing param to get the userinfo and fix the callback - ([8ca8273](https://github.com/batleforc/proxyauthK8s/commit/8ca82739a0c34ec8e4cb0b7c5167e9c7ab5e28de)) - Max Batleforc
- (**front**) Setup auth guard and requiresAuth meta descriptor - ([7a6e64e](https://github.com/batleforc/proxyauthK8s/commit/7a6e64e5aff01e048b724fe5d2ccc151a823725f)) - Max Batleforc
- (**front**) add pinia - ([cfded20](https://github.com/batleforc/proxyauthK8s/commit/cfded201fdee6b92e3e80d3baf018e76f6b41cbf)) - Max Batleforc
- (**front**) create front app - ([59055e7](https://github.com/batleforc/proxyauthK8s/commit/59055e784f09bb2b2f2dddad64dbca5f545414d4)) - Max Batleforc
- (**front-api**) add gen swagger api for front-api - ([e34a1ba](https://github.com/batleforc/proxyauthK8s/commit/e34a1bad2ac9e6f544f5c718f6afef7248204978)) - Max Batleforc
- (**front/back**) Mise en place de l'authentification dédié clusters - ([cfdff15](https://github.com/batleforc/proxyauthK8s/commit/cfdff15d4e0ac37aece234d5f0067ead1829457d)) - Max Batleforc
- (**hooks**) Enforce linting at each commit - ([8fe474f](https://github.com/batleforc/proxyauthK8s/commit/8fe474fc612d6faa0655d968404267cd6629716a)) - Max Batleforc
- (**kubectl_proxyauth**) move cli to it's lib like the rest - ([5a1015a](https://github.com/batleforc/proxyauthK8s/commit/5a1015a65a898d75269ca95435dbbd16e1795949)) - Max Batleforc
- (**kubectl_proxyauth**) prepare async func - ([69a95c9](https://github.com/batleforc/proxyauthK8s/commit/69a95c919fefa5b8ceca841e21449ffe66cc490c)) - Max Batleforc
- (**kubectl_proxyauth**) include banner in help message ++ - ([12fb8f6](https://github.com/batleforc/proxyauthK8s/commit/12fb8f67f6381fbe92350669c84ad92add178839)) - Max Batleforc
- (**kubectl_proxyauth**) Working context handling and switching - ([bc6b6f9](https://github.com/batleforc/proxyauthK8s/commit/bc6b6f918887400588d0ecf9d8f97a09e379eeba)) - Max Batleforc
- (**kubectl_proxyauth**) Create a config file and WIP the spec of the - ([5d40c04](https://github.com/batleforc/proxyauthK8s/commit/5d40c046f2b6d3dbf4050f31a56bbba1812190bd)) - Max Batleforc
- (**kubectl_proxyauth**) Evol Spec ++ define Kubeconfig exec part - ([1d2daae](https://github.com/batleforc/proxyauthK8s/commit/1d2daae7cd7e16cfe4791ea1166ca4fee7e70bfb)) - Max Batleforc
- (**kubectl_proxyauth**) Setup Tracing and wip on context setup - ([eba7c51](https://github.com/batleforc/proxyauthK8s/commit/eba7c51c12862d32364ff346807262242ca7b14b)) - Max Batleforc
- (**kubectl_proxyauth**) add check for localhost - ([56f22db](https://github.com/batleforc/proxyauthK8s/commit/56f22db3301a1b9d3d77d43eb64375861fc239d6)) - Max Batleforc
- (**kubectl_proxyauth**) changed auth to allow it to be kubectl - ([d377730](https://github.com/batleforc/proxyauthK8s/commit/d37773006674b1039e96ebf62716aee35e6fbdc3)) - Max Batleforc
- (**kubectl_proxyauth**) Write spec and start work on the kubectl - ([c3dcaca](https://github.com/batleforc/proxyauthK8s/commit/c3dcaca869097b02569cfbc67c49ca72002a6f63)) - Max Batleforc
- (**readme**) add more info related on the project - ([b0468d7](https://github.com/batleforc/proxyauthK8s/commit/b0468d75cdc884d3257deddac7b5a8753d31771d)) - Max Batleforc
- (**redirect**) add more trace like the exit code - ([ed179ec](https://github.com/batleforc/proxyauthK8s/commit/ed179ecd3d131adc3ee22e59b145d12e1d03e1f6)) - Max Batleforc
- (**redis**) move from r2d2 to Deadpool, would be a headack perf wise - ([c876e98](https://github.com/batleforc/proxyauthK8s/commit/c876e9864b880e63cf024f2678479a5f0130f7d7)) - Max Batleforc
- (**rust-api**) projetitor does not handle swagger 3.1 - ([dfe2c95](https://github.com/batleforc/proxyauthK8s/commit/dfe2c95b7c5a5317a02893d21b448fc0fb479c77)) - Max Batleforc
- (**state**) prepare global state - ([2553ec6](https://github.com/batleforc/proxyauthK8s/commit/2553ec6a3442f103f9079725061713ef64bbb196)) - Max Batleforc
- (**swagger**) dont create front-api for route that should not be called - ([56e5341](https://github.com/batleforc/proxyauthK8s/commit/56e53411efb5f34b6c6500613293e2c1a8ec89db)) - Max Batleforc
- (**taskfile**) add lab taskfile - ([2ab4183](https://github.com/batleforc/proxyauthK8s/commit/2ab4183295a3bc14bf597b80c057c0ea5bdc6047)) - Max Batleforc
- (**tracing**) IT WORKS - ([8b138e0](https://github.com/batleforc/proxyauthK8s/commit/8b138e0b2382e5281d06e4792f0789283543693a)) - Max Batleforc
- (**tracing**) WIP on tracing - ([97d883e](https://github.com/batleforc/proxyauthK8s/commit/97d883e99515b91bb0530278b473cab2d08e52f3)) - Max Batleforc
- (**tracing**) WIP ATM it does not work - ([56536b8](https://github.com/batleforc/proxyauthK8s/commit/56536b8abdaf311bd3ec07f01f507fe20f36fe7a)) - Max Batleforc
- (**upgrade**) rsa ? - ([3918da1](https://github.com/batleforc/proxyauthK8s/commit/3918da190f8e70917bdcd66a1551145b90a8a865)) - Max Batleforc
- (**upgrade**) Work on it - ([f4fbeda](https://github.com/batleforc/proxyauthK8s/commit/f4fbeda00488734a96b1f1e1dc86613ea961cf93)) - Max Batleforc
- (**vscode**) add taskfile extensions - ([9d92bb3](https://github.com/batleforc/proxyauthK8s/commit/9d92bb337d188eec98a4d9f88c10f5ba11ce0145)) - Max Batleforc
- change cog - ([40bf9f3](https://github.com/batleforc/proxyauthK8s/commit/40bf9f399a80c6b5bddac2ee97621d3055edaae7)) - Max Batleforc
- upgrade image - ([69194d4](https://github.com/batleforc/proxyauthK8s/commit/69194d4a74a8909d73fc21fdef1c7d41ed547c0d)) - Max Batleforc
- upgrade cargo - ([0c84f1f](https://github.com/batleforc/proxyauthK8s/commit/0c84f1f096d19e8dfbce9957107fdf89cb683eaa)) - Max Batleforc
- upgrade - ([5e50bfe](https://github.com/batleforc/proxyauthK8s/commit/5e50bfe0061568e1bbbbac834eb0127c94aed6ce)) - Max Batleforc
- setup formater ! - ([5632239](https://github.com/batleforc/proxyauthK8s/commit/5632239cc939de17641d6291d74000bc7a8bdbfa)) - Max Batleforc
- remove translation - ([db1ec69](https://github.com/batleforc/proxyauthK8s/commit/db1ec69e92b62c307b136c685ddf2bab84cf0098)) - Max Batleforc
- add new config - ([03065ed](https://github.com/batleforc/proxyauthK8s/commit/03065ed1b9f1fef97441808a6064ea7dc881aa1d)) - Max Batleforc
- upgrade front - ([b82df22](https://github.com/batleforc/proxyauthK8s/commit/b82df2213793f080bf833d82110afb5b027b5926)) - Max Batleforc
- work on using eslint as formater - ([a9e3c31](https://github.com/batleforc/proxyauthK8s/commit/a9e3c31021264da263d16f9ca901f71694d2e8ec)) - Max Batleforc
- wip - ([6f2532a](https://github.com/batleforc/proxyauthK8s/commit/6f2532a394df7e8982ae5503589cdcfe8f10894a)) - Max Batleforc
- Work on docs and disable ban ts comment in case of generated lib - ([2da7a49](https://github.com/batleforc/proxyauthK8s/commit/2da7a498605007a7cf89ac3f7c6a60808d1689d2)) - Max Batleforc
- upgrade sarif - ([186cbdb](https://github.com/batleforc/proxyauthK8s/commit/186cbdbdc54ded899f25a940955682100cef04b2)) - Max Batleforc
- wip - ([0ca619e](https://github.com/batleforc/proxyauthK8s/commit/0ca619e1f3106e26192065d24071df12c2c7fd8e)) - Max Batleforc
- make the api-lint workflows disapatch - ([b53d1f4](https://github.com/batleforc/proxyauthK8s/commit/b53d1f42c46adbb8cc54eeb75735c22f99922682)) - Max Batleforc
- clean old config - ([930947d](https://github.com/batleforc/proxyauthK8s/commit/930947d56b10998c2775440a85a812645434d141)) - Max Batleforc
- Upgrade to eslintrc flat - ([57e7522](https://github.com/batleforc/proxyauthK8s/commit/57e75221e10eb6c7736b939703f73d9acfa4bcf3)) - Max Batleforc
- Add Stance on IA ++ Start working on tag action - ([8db3976](https://github.com/batleforc/proxyauthK8s/commit/8db397698aa358ec4e521908124f535fd4a1b373)) - Max Batleforc
- validate K3D Oidc configuration - ([b81590f](https://github.com/batleforc/proxyauthK8s/commit/b81590fe4d45633c0ead3a940ac395ecaf3dea5c)) - Max Batleforc
- Add new ClusterNoSSOView to allow getting kubeconfig when not - ([1dffaca](https://github.com/batleforc/proxyauthK8s/commit/1dffaca5009dc5ad54c4e9fe16ee103328783f20)) - Max Batleforc
- wip - ([41182f8](https://github.com/batleforc/proxyauthK8s/commit/41182f877b7c802b305a3e3061a8e14c35298cdc)) - Max Batleforc
- Mise en place Lease + Reconcile incrémental - ([69ca227](https://github.com/batleforc/proxyauthK8s/commit/69ca2274375d7f78e1034bbd5d19b53ec45da3f3)) - Max Batleforc
- add redis to schema - ([a326750](https://github.com/batleforc/proxyauthK8s/commit/a3267508ce8f6338ee86f337ad40eb4aadeee9a9)) - Max Batleforc
- add démo - ([c75454e](https://github.com/batleforc/proxyauthK8s/commit/c75454e8a6a6ba1c043b475ae24af981f85d4291)) - Max Batleforc
- Handle ConfigMap as a source for cert - ([15d2d26](https://github.com/batleforc/proxyauthK8s/commit/15d2d2672b378fd04c6cb9dcc8e918f3c80e5804)) - Max Batleforc
- upgrade again - ([32f89d2](https://github.com/batleforc/proxyauthK8s/commit/32f89d2b78b9bda72f8dd9230b2edb59704c72ed)) - Max Batleforc
- upgrade node - ([5d921a2](https://github.com/batleforc/proxyauthK8s/commit/5d921a2ed13bcda88a44eb17cb7c954e9bd8840c)) - Max Batleforc
- Handle K3D - ([ae15362](https://github.com/batleforc/proxyauthK8s/commit/ae153626a3130eb0e639e232750481cd92dd0f60)) - Max Batleforc
- wip k3d - ([b039181](https://github.com/batleforc/proxyauthK8s/commit/b039181097f9deb38c72db01f451be6edd7879d1)) - Max Batleforc
- upgrade package - ([fef1755](https://github.com/batleforc/proxyauthK8s/commit/fef17553dd98bcbb9f05e97526a8d2f4b20eae2d)) - Max Batleforc
- remove rollup mapp package - ([6d3c63e](https://github.com/batleforc/proxyauthK8s/commit/6d3c63eff874a8b0db07984eb0ef550419450c39)) - Max Batleforc
- init work on having k3d - ([38bc9ba](https://github.com/batleforc/proxyauthK8s/commit/38bc9baf7f4faf28d1bdad8f61fcdf3ad03e463a)) - Max Batleforc
- Mise en place validation technique des instance CRD avant - ([20cd907](https://github.com/batleforc/proxyauthK8s/commit/20cd9076c8bc0750435f17fc30d3db04bfa2572b)) - Max Batleforc
- Create get_user_info based on kubernetes claim - ([f57dba2](https://github.com/batleforc/proxyauthK8s/commit/f57dba2f76be8c7f00e6233d142323ab767ed274)) - Max Batleforc
- extend api with validator against - ([2e7f929](https://github.com/batleforc/proxyauthK8s/commit/2e7f92920af5675a35a716af5c1123705ec8ba5a)) - Max Batleforc
- wip - ([99dbdad](https://github.com/batleforc/proxyauthK8s/commit/99dbdade961931e05e6825479984e91ab621619d)) - Max Batleforc
- make a try - ([c64dcfe](https://github.com/batleforc/proxyauthK8s/commit/c64dcfe8faf8c4c49fc67d392dd7dedd41d7627b)) - Max Batleforc
- wip - ([37b768d](https://github.com/batleforc/proxyauthK8s/commit/37b768da5e2ec4d8536dcbec3f114a67cce25e4a)) - Max Batleforc
- wip - ([bf7baee](https://github.com/batleforc/proxyauthK8s/commit/bf7baeeeda46a4be0a143b58434c9f0de6efa495)) - Max Batleforc
- wip - ([a85a6ca](https://github.com/batleforc/proxyauthK8s/commit/a85a6ca8f3083393e9842ab32773af69910038b9)) - Max Batleforc
- wipo - ([1689ba7](https://github.com/batleforc/proxyauthK8s/commit/1689ba77840266c2a554707c336c4423e49bba85)) - Max Batleforc
- wip new devfile - ([8c90829](https://github.com/batleforc/proxyauthK8s/commit/8c908297ba1b3fffaecf0b86aa30d3412ad65b92)) - Max Batleforc
- wip upgrade - ([f7b5606](https://github.com/batleforc/proxyauthK8s/commit/f7b560636a9d3afdd4dc19a799213951564f4a3b)) - Max Batleforc
- upgrade 2 - ([4eea8b4](https://github.com/batleforc/proxyauthK8s/commit/4eea8b48451e5827af9e275113207aad7f4e55dd)) - Max Batleforc
- Upgrade package - ([5a15174](https://github.com/batleforc/proxyauthK8s/commit/5a15174f3b17d3fe3eba8ef05f06778b289e9e20)) - Max Batleforc
- push CODEOWNERS - ([982d301](https://github.com/batleforc/proxyauthK8s/commit/982d301ca08b87f57e1b10cd5e15b7e48791b9ae)) - Max Batleforc
- Add PR template and CONTRIBUTING - ([26e4cec](https://github.com/batleforc/proxyauthK8s/commit/26e4cec3cffcba643f34e74488e8092b064031b5)) - Max Batleforc
- add issue template - ([eb33e38](https://github.com/batleforc/proxyauthK8s/commit/eb33e38dcc2fe94e8e870004e39120b670e3fe64)) - Max Batleforc
- remove old workflow - ([2be2d54](https://github.com/batleforc/proxyauthK8s/commit/2be2d54827dbf0b657d24d85f60539bdb0956853)) - Max Batleforc
- remove codeql old - ([7d9d7a4](https://github.com/batleforc/proxyauthK8s/commit/7d9d7a43adac4a5734d2c7fa79178b41f4b99890)) - Max Batleforc
- wip - ([edb1ba9](https://github.com/batleforc/proxyauthK8s/commit/edb1ba9f06ae28461e24c0a6815f40d1bdf88e2d)) - Max Batleforc
- Apply security check - ([555d3c3](https://github.com/batleforc/proxyauthK8s/commit/555d3c3dc3a4090ee56142568816f1f888ae9c41)) - Maxime
- Update .github/workflows/codeql.yml - ([e032284](https://github.com/batleforc/proxyauthK8s/commit/e032284cf5f60f111ac6d335693c93fd3de987a9)) - Maxime
- Update .github/workflows/dependency-review.yml - ([00f3e4c](https://github.com/batleforc/proxyauthK8s/commit/00f3e4ccb654d2c4cbcd399d6d6435540d7b4e69)) - Maxime
- Update .github/workflows/dependency-review.yml - ([b69243f](https://github.com/batleforc/proxyauthK8s/commit/b69243fe839af6e0397d8c940e52d46a15956236)) - Maxime
- [StepSecurity] Apply security best practices - ([a79b71a](https://github.com/batleforc/proxyauthK8s/commit/a79b71a2c8a72ff21c5a23d4181201ec8ad5229a)) - StepSecurity Bot
- Move module in subfolder - ([239d73c](https://github.com/batleforc/proxyauthK8s/commit/239d73c70ca0f9181b82fb2b8dbd9e6fbfbe87af)) - Max Batleforc
- wip - ([ab3815a](https://github.com/batleforc/proxyauthK8s/commit/ab3815a51032b05c94e8ea78864d39696b3cea05)) - Max Batleforc
- wip - ([6bd83b3](https://github.com/batleforc/proxyauthK8s/commit/6bd83b39f6381fb22c7967a6f704efd2797d4ca7)) - Max Batleforc
- Upgrade package and add hooks - ([523dec1](https://github.com/batleforc/proxyauthK8s/commit/523dec1feb88daae1017df50220c6d8d30f2f5f4)) - Max Batleforc
- extract cli main logic to lib and add to ui ClusterCallbackView a - ([2847b5b](https://github.com/batleforc/proxyauthK8s/commit/2847b5b125c846c265bf13ec7486b14d4d11437d)) - Max Batleforc
- wip - ([cb502aa](https://github.com/batleforc/proxyauthK8s/commit/cb502aaa577c6b9d724dda19756ddde242912ae7)) - Max Batleforc
- add gitignore - ([0cae86f](https://github.com/batleforc/proxyauthK8s/commit/0cae86f6ac9d0e3c0e706de32f9854b47efbf7c0)) - Max Batleforc
- make a try - ([4ef6134](https://github.com/batleforc/proxyauthK8s/commit/4ef613467536444e7b0935f9e3d2a89c37cb60e5)) - Max Batleforc
- wip cicd - ([4be6662](https://github.com/batleforc/proxyauthK8s/commit/4be666280b2aeb941c0c6e4185e7963ff5fb5ef2)) - Max Batleforc
- wip cicd - ([3cbcdf6](https://github.com/batleforc/proxyauthK8s/commit/3cbcdf683706fd0371bcc4e7a0f1a5548c7028b8)) - Max Batleforc
- add helm dependency build - ([b146504](https://github.com/batleforc/proxyauthK8s/commit/b146504d4ff252fa3a736176dc2000bb833a4e4e)) - Max Batleforc
- wip - ([ff0ad98](https://github.com/batleforc/proxyauthK8s/commit/ff0ad9892744c5b26efdcd366f3fbb145f444c4f)) - Max Batleforc
- wip - ([40b5aa8](https://github.com/batleforc/proxyauthK8s/commit/40b5aa8ec8bbe53feeecb214e59ea11d95af6856)) - Max Batleforc
- make a try - ([bac9ff8](https://github.com/batleforc/proxyauthK8s/commit/bac9ff88fa67b1142059fdf276390177f4724301)) - Max Batleforc
- try another changes - ([5c9e684](https://github.com/batleforc/proxyauthK8s/commit/5c9e6844e3cde2d99d7e718b18a2f7cfd577ea86)) - Max Batleforc
- wip on merge guide - ([45bc6fd](https://github.com/batleforc/proxyauthK8s/commit/45bc6fd348ee40854b108170783fbe78538ec518)) - Max Batleforc
- add git release guide - ([a797138](https://github.com/batleforc/proxyauthK8s/commit/a797138d35b94f633a1afdbff0a4b9a32c404319)) - Max Batleforc
- wip - ([f1bc962](https://github.com/batleforc/proxyauthK8s/commit/f1bc96233b75c94c4c919a571b1d3e27e6bac6de)) - Max Batleforc
- wip - ([73c3bc6](https://github.com/batleforc/proxyauthK8s/commit/73c3bc64f5d53c28757df6cc49162556bf425afd)) - Max Batleforc
- wip ci/cd - ([3f878fc](https://github.com/batleforc/proxyauthK8s/commit/3f878fc77cbff992c148d7508abfe23436d7a647)) - Max Batleforc
- wip - ([1ec1946](https://github.com/batleforc/proxyauthK8s/commit/1ec194650fa051d52b8a75f63da714974664d788)) - Max Batleforc
- wip - ([80ac250](https://github.com/batleforc/proxyauthK8s/commit/80ac250c4f0c4503ddfe7653e41ec6955f352975)) - Max Batleforc
- wip - ([b2aed74](https://github.com/batleforc/proxyauthK8s/commit/b2aed74e08c7ee4f10ca3674781e7e5a3adf71dc)) - Max Batleforc
- wip - ([2c70225](https://github.com/batleforc/proxyauthK8s/commit/2c70225f2bb26a49b70d23ae50b0c129f314fb68)) - Max Batleforc
- remove detailed commit validation - ([e1e996f](https://github.com/batleforc/proxyauthK8s/commit/e1e996feee862231e3faa5d967311924175e0d57)) - Max Batleforc
- wip - ([28f20aa](https://github.com/batleforc/proxyauthK8s/commit/28f20aa979de8f9419029c5cc6940c00ec457682)) - Max Batleforc
- wip - ([ca206af](https://github.com/batleforc/proxyauthK8s/commit/ca206afa5f5bd54a1973c69f00911fdbdf6c96b6)) - Max Batleforc
- make it work ! - ([2023efb](https://github.com/batleforc/proxyauthK8s/commit/2023efb92273bdf0eb0d4cfbdead54fecc2a55d0)) - Max Batleforc
- wip - ([f9ea72c](https://github.com/batleforc/proxyauthK8s/commit/f9ea72c3d2303c4048c6496917f8177327e542ed)) - Max Batleforc
- wip - ([b81014f](https://github.com/batleforc/proxyauthK8s/commit/b81014facb6fe605fe089d7a3db1a6420a2e92ad)) - Max Batleforc
- wip - ([8b3a7c9](https://github.com/batleforc/proxyauthK8s/commit/8b3a7c9c0a13642f19426e496785f8945589ad66)) - Max Batleforc
- make redis secret or env - ([95633ff](https://github.com/batleforc/proxyauthK8s/commit/95633ffffed608376409251a7d3418f20c1d80ea)) - Max Batleforc
- create basic helm chart - ([6e44a25](https://github.com/batleforc/proxyauthK8s/commit/6e44a25fe83f86daaf958b9658769c5f90cd005e)) - Max Batleforc
- make it build and work on having a proper compose to try it all - ([a43760d](https://github.com/batleforc/proxyauthK8s/commit/a43760dbec77e4110799e2b6a2257e8eda409527)) - Max Batleforc
- wip - ([82b21dd](https://github.com/batleforc/proxyauthK8s/commit/82b21dd9ee6bc87bc34b05daa80434c6d1d6ce04)) - Max Batleforc
- add element in crd or api model - ([984c44e](https://github.com/batleforc/proxyauthK8s/commit/984c44e5bb85dd0ebd6540fce17f113ac3d44c15)) - Max Batleforc
- Mise en place communication authentifier entre le front/back - ([c2a619c](https://github.com/batleforc/proxyauthK8s/commit/c2a619cc7df0fe6e99cb09ddf6ddfc1b044437c7)) - Max Batleforc
- wip on tracing and OIDC - ([5b39d84](https://github.com/batleforc/proxyauthK8s/commit/5b39d8422ce094a5a208e7f900a4d2e5a3505aab)) - Max Batleforc
- WIP on bringing OIDC to the talos cluster - ([f52e957](https://github.com/batleforc/proxyauthK8s/commit/f52e957ac1d74c6dd25373189592525db39c99b6)) - Max Batleforc
- auth work, next step "mtls" auth - ([97c5522](https://github.com/batleforc/proxyauthK8s/commit/97c55226a5b253144a4866b9e9c35ef1a207fd1b)) - Max Batleforc
- WIP HTTPs - ([2b2a3d2](https://github.com/batleforc/proxyauthK8s/commit/2b2a3d2709f06c8f18ecffc4b2e427afc59a53d4)) - Max Batleforc
- send reqwest to the upstream server - ([1533924](https://github.com/batleforc/proxyauthK8s/commit/153392455b4b1d5c5ca7dd14475a554131bd55b1)) - Max Batleforc
- init base work of redirect - ([8c4af60](https://github.com/batleforc/proxyauthK8s/commit/8c4af609cd51161d9c06f34e61f6cadb71081e14)) - Max Batleforc
- wip - ([0563580](https://github.com/batleforc/proxyauthK8s/commit/056358045f831d3ad138c9eb5b3a0541b9d0d137)) - Max Batleforc
- WIP add authelia support - ([13e4321](https://github.com/batleforc/proxyauthK8s/commit/13e4321214811a31025425dc3772cccb3243dd4a)) - Max Batleforc
- wip - ([9f0803c](https://github.com/batleforc/proxyauthK8s/commit/9f0803ce24ae0c6f05bd371cb78432731929050d)) - Max Batleforc
- Add Redis and go back to trace, something is missing - ([1ca2559](https://github.com/batleforc/proxyauthK8s/commit/1ca2559bc7039ca18d0c295d609b64f2023361bd)) - Max Batleforc
- wip - ([2e26ebc](https://github.com/batleforc/proxyauthK8s/commit/2e26ebcff7c63314c6eeb7279d3690515aaedea5)) - Max Batleforc
- add a local talos cluster - ([598dee8](https://github.com/batleforc/proxyauthK8s/commit/598dee838389e7911d7987f594e35ae39112f8bb)) - Max Batleforc
- Init ProxyAuthK8s project - ([77971ab](https://github.com/batleforc/proxyauthK8s/commit/77971abf9a410d095a3bbdf5204844ebf851c693)) - Max Batleforc
- Initial commit - ([7e79a7c](https://github.com/batleforc/proxyauthK8s/commit/7e79a7cc31943ac20adc0a58a6b20659c83f1efa)) - Max Batleforc
#### Bug Fixes
- (**ci**) wip - ([e2a1bcb](https://github.com/batleforc/proxyauthK8s/commit/e2a1bcbbe014e7f229bb7f4997007cee00b8e646)) - Max Batleforc
- (**ci**) wip - ([5d54a69](https://github.com/batleforc/proxyauthK8s/commit/5d54a69cab657ef4786ff607e2dbc2e20efcc4fb)) - Max Batleforc
- (**ci**) run migration migrate for nx - ([e89508b](https://github.com/batleforc/proxyauthK8s/commit/e89508b30c262d750e75f1438bb267f39673bfd2)) - Max Batleforc
- (**client_api**) auto clippy - ([74508ed](https://github.com/batleforc/proxyauthK8s/commit/74508ed0e24969cba7519aeb8adfbde3d573c38c)) - Max Batleforc
- (**controller**) path used in manifest - ([7ec07f9](https://github.com/batleforc/proxyauthK8s/commit/7ec07f996525c53c4a6e608d32d123b9a25896b9)) - Max Batleforc
- (**devfile**) dbgate mount path - ([6df2361](https://github.com/batleforc/proxyauthK8s/commit/6df23617fd44192a4a4c3f1c09af4adaa67e6161)) - Max Batleforc
- (**utoipa**) automate the generation of Swagger api - ([a4a4ba0](https://github.com/batleforc/proxyauthK8s/commit/a4a4ba079244a95c4614548d57014a4a18d2d8bb)) - Max Batleforc
- make the sbom action pinned - ([23e6522](https://github.com/batleforc/proxyauthK8s/commit/23e6522d51bdf0c9c2ef5bd205cd65368a5aa98e)) - Max Batleforc
- apply spectral recommendation - ([1a7dab9](https://github.com/batleforc/proxyauthK8s/commit/1a7dab91ba43d46516db424e81bf4c8360c20a0b)) - Max Batleforc
- Infinite reconcile loop (i'm dub xD) - ([3b2b74e](https://github.com/batleforc/proxyauthK8s/commit/3b2b74e8f4e5fd0c8ada32904b0e6c8050e59dfd)) - Max Batleforc
- wiiiippp - ([2867619](https://github.com/batleforc/proxyauthK8s/commit/2867619906853397263017240ed3137414f80f2f)) - Max Batleforc
- no cpu - ([fab7cc6](https://github.com/batleforc/proxyauthK8s/commit/fab7cc625d1d2b7a5811a5c8c1926e4ef27dcb8d)) - Max Batleforc
- wrong parm - ([35e953c](https://github.com/batleforc/proxyauthK8s/commit/35e953cc731104770c84c3e932802293e04c2173)) - Max Batleforc
- swagger part - ([65fbfae](https://github.com/batleforc/proxyauthK8s/commit/65fbfae0e9b29c2ee7ad93eb3d5033d3acbef4d6)) - Max Batleforc
- wip - ([96a2ef0](https://github.com/batleforc/proxyauthK8s/commit/96a2ef0ce7e88a9a8a6ca187b7e185b057c36908)) - Max Batleforc
- wip - ([49a4e95](https://github.com/batleforc/proxyauthK8s/commit/49a4e9562bb8a56aa7a342b4c75b81c3c8bbf5b6)) - Max Batleforc
- it should work - ([c9067a6](https://github.com/batleforc/proxyauthK8s/commit/c9067a63ebf30b712f3283dba46a8d493182daf0)) - Max Batleforc
#### Build system
- (**deps**) bump github/codeql-action from 4.32.5 to 4.32.6 - ([1e914a4](https://github.com/batleforc/proxyauthK8s/commit/1e914a40cd0e8f456cb256615212759e7dbb46d1)) - Maxime
- (**deps**) bump github/codeql-action from 4.32.5 to 4.32.6 - ([c081848](https://github.com/batleforc/proxyauthK8s/commit/c0818482d0cb2f20d6ab48aae7148c026d3d5ace)) - dependabot[bot]
- (**deps**) bump docker/setup-buildx-action from 3.12.0 to 4.0.0 - ([be8ea12](https://github.com/batleforc/proxyauthK8s/commit/be8ea1218700a98ca4fc9cbab426a81eb4679315)) - Maxime
- (**deps**) bump rustsec/audit-check from 1.4.1 to 2.0.0 - ([64235b4](https://github.com/batleforc/proxyauthK8s/commit/64235b447149a3b4406fb8be52f74f809a29e970)) - Maxime
- (**deps**) bump actions/setup-node from 4.4.0 to 6.3.0 - ([6068378](https://github.com/batleforc/proxyauthK8s/commit/60683784b0801eeeb507b4313ef92950a560e1b0)) - Maxime
- (**deps**) bump docker/setup-buildx-action from 3.12.0 to 4.0.0 - ([98276cd](https://github.com/batleforc/proxyauthK8s/commit/98276cd941a3fee3adf7f16d053ff8b918f5532d)) - dependabot[bot]
- (**deps**) bump rustsec/audit-check from 1.4.1 to 2.0.0 - ([7a8296f](https://github.com/batleforc/proxyauthK8s/commit/7a8296fe08139562e28bd36bbdd5ed44e96a8f30)) - dependabot[bot]
- (**deps**) bump actions/setup-node from 4.4.0 to 6.3.0 - ([b29069d](https://github.com/batleforc/proxyauthK8s/commit/b29069db92bc6797894d51995be536e06fb28e28)) - dependabot[bot]
- (**deps**) bump docker/login-action from 3.6.0 to 4.0.0 - ([5b1b280](https://github.com/batleforc/proxyauthK8s/commit/5b1b280ac650996205d739ffb6a27d2a12ed78db)) - Maxime
- (**deps**) bump github/codeql-action from 4.32.4 to 4.32.5 - ([48b8fed](https://github.com/batleforc/proxyauthK8s/commit/48b8fed7550dca8a4db6af006b8d7f1551ec955c)) - Maxime
- (**deps**) bump actions/checkout from 4.3.1 to 6.0.2 - ([8579a9b](https://github.com/batleforc/proxyauthK8s/commit/8579a9b620a545d57730b16516f70e4b65e5205c)) - Maxime
- (**deps**) bump actions/checkout from 4.3.1 to 6.0.2 - ([d3ab8bd](https://github.com/batleforc/proxyauthK8s/commit/d3ab8bdc86f77336860537115620c90def3b490f)) - dependabot[bot]
- (**deps**) bump github/codeql-action from 4.32.4 to 4.32.5 - ([c2336b9](https://github.com/batleforc/proxyauthK8s/commit/c2336b99c7a4e4badc486c42a52a3e982209e5d8)) - dependabot[bot]
- (**deps**) bump actions/dependency-review-action from 4.8.3 to 4.9.0 - ([183711c](https://github.com/batleforc/proxyauthK8s/commit/183711cc207eeaa1a9e104c53b053f705530f46c)) - Maxime
- (**deps**) bump actions/upload-artifact from 6.0.0 to 7.0.0 - ([06ee1d4](https://github.com/batleforc/proxyauthK8s/commit/06ee1d400e14b5c3083866c768186026e37c5348)) - Maxime
- (**deps**) bump actions/upload-artifact from 6.0.0 to 7.0.0 - ([305e474](https://github.com/batleforc/proxyauthK8s/commit/305e474e8462b651a68b2753c5444e09e7e5652f)) - dependabot[bot]
- (**deps**) bump actions/dependency-review-action from 4.8.3 to 4.9.0 - ([050b266](https://github.com/batleforc/proxyauthK8s/commit/050b266e3243a9c7f9d217b4d2af9e83da448e90)) - dependabot[bot]
- (**deps**) bump docker/login-action from 3.6.0 to 4.0.0 - ([0308f86](https://github.com/batleforc/proxyauthK8s/commit/0308f8647cb49dfa70d442ba85e267837d9b0649)) - dependabot[bot]
- (**deps**) bump actions/dependency-review-action from 4.8.2 to 4.8.3 - ([475edcf](https://github.com/batleforc/proxyauthK8s/commit/475edcf3a32d9e584c1e9902e87c73f01aeee9fc)) - Maxime
- (**deps**) bump github/codeql-action from 4.32.3 to 4.32.4 - ([e7b60db](https://github.com/batleforc/proxyauthK8s/commit/e7b60dbc7bc806856888a82d34dc18897414a5c6)) - Maxime
- (**deps**) bump actions/upload-artifact from 6.0.0 to 7.0.0 - ([264b661](https://github.com/batleforc/proxyauthK8s/commit/264b6615fc10ecfcad557b95a771461486e5dc18)) - Maxime
- (**deps**) bump actions/upload-artifact from 6.0.0 to 7.0.0 - ([0882020](https://github.com/batleforc/proxyauthK8s/commit/08820207063b98cab2bafa22f1dfff21bb074520)) - dependabot[bot]
- (**deps**) bump github/codeql-action from 4.32.3 to 4.32.4 - ([c4a10cd](https://github.com/batleforc/proxyauthK8s/commit/c4a10cd2fe41015ae7d502b8338ebe617af1cf47)) - dependabot[bot]
- (**deps**) bump actions/dependency-review-action from 4.8.2 to 4.8.3 - ([a93e794](https://github.com/batleforc/proxyauthK8s/commit/a93e79415eca5881394745f2733278abdccd3296)) - dependabot[bot]
- (**deps**) bump lodash from 4.17.21 to 4.17.23 - ([7fd6d6e](https://github.com/batleforc/proxyauthK8s/commit/7fd6d6e4eeef9cdb42cdbd1f7ba54c239582e002)) - Maxime
- (**deps**) bump github/codeql-action from 4.31.9 to 4.32.3 - ([ff32191](https://github.com/batleforc/proxyauthK8s/commit/ff32191b23192d90b024a6c116d4b8d03fafa11b)) - Maxime
- (**deps**) bump actions/setup-node from 6.1.0 to 6.2.0 - ([d30fbb8](https://github.com/batleforc/proxyauthK8s/commit/d30fbb8744d52c01684a83a785ced01ac18c15b7)) - Maxime
- (**deps**) bump actions/checkout from 6.0.1 to 6.0.2 - ([0991854](https://github.com/batleforc/proxyauthK8s/commit/09918544e34c7e436674efcce1a34b50022309a5)) - Maxime
- (**deps**) bump lodash from 4.17.21 to 4.17.23 - ([44012c2](https://github.com/batleforc/proxyauthK8s/commit/44012c21cb7dc14948e29453d01e2dd73d0558ef)) - dependabot[bot]
- (**deps**) bump github/codeql-action from 4.31.9 to 4.32.3 - ([962c39c](https://github.com/batleforc/proxyauthK8s/commit/962c39c4389a9ae6c45e90eefe5192d9438f0c1f)) - dependabot[bot]
- (**deps**) bump actions/checkout from 6.0.1 to 6.0.2 - ([3049f6a](https://github.com/batleforc/proxyauthK8s/commit/3049f6a384df3154bec27b5ebd137a51ec869650)) - dependabot[bot]
- (**deps**) bump actions/setup-node from 6.1.0 to 6.2.0 - ([4e996f1](https://github.com/batleforc/proxyauthK8s/commit/4e996f1d67b2efa739a907f2675efcff44a833c3)) - dependabot[bot]
- (**deps**) bump actions/dependency-review-action from 4.2.5 to 4.8.2 - ([2d35fcc](https://github.com/batleforc/proxyauthK8s/commit/2d35fcc458304eb9c83e697efee6500e37a854d1)) - Maxime
- (**deps**) bump actions/checkout from 4.2.2 to 6.0.1 - ([fce491d](https://github.com/batleforc/proxyauthK8s/commit/fce491d59238b9f0c7b5f7a4c65505b52b245a11)) - Maxime
- (**deps**) bump actions/checkout from 4.2.2 to 6.0.1 - ([8c3da9e](https://github.com/batleforc/proxyauthK8s/commit/8c3da9e669a5b0b8b1996e5d40e431e6dee1beca)) - dependabot[bot]
- (**deps**) bump actions/upload-artifact from 4.6.1 to 6.0.0 - ([5a7b2ad](https://github.com/batleforc/proxyauthK8s/commit/5a7b2adcb86e5ebab0332f8a65edbd62d7d7fa83)) - Maxime
- (**deps**) bump actions/setup-node from 4.4.0 to 6.1.0 - ([5492c7a](https://github.com/batleforc/proxyauthK8s/commit/5492c7ac2ab926a3a68a79bda043934c758d175e)) - Maxime
- (**deps**) bump actions/dependency-review-action from 4.2.5 to 4.8.2 - ([155823f](https://github.com/batleforc/proxyauthK8s/commit/155823fe2ff4bc8023337fe301dca362ebe2e378)) - dependabot[bot]
- (**deps**) bump actions/setup-node from 4.4.0 to 6.1.0 - ([771b2ed](https://github.com/batleforc/proxyauthK8s/commit/771b2ed023d4fa6a7ad522ce3c5e34f13ed7a510)) - dependabot[bot]
- (**deps**) bump actions/upload-artifact from 4.6.1 to 6.0.0 - ([ca41d57](https://github.com/batleforc/proxyauthK8s/commit/ca41d57348459873533ddb1e67f72eb77d774d32)) - dependabot[bot]
- (**deps-dev**) bump @types/node from 25.3.3 to 25.3.5 - ([7132199](https://github.com/batleforc/proxyauthK8s/commit/7132199952139b9408a85d6a1acf1339e538c03f)) - Maxime
- (**deps-dev**) bump @types/node from 25.3.3 to 25.3.5 - ([0ebe238](https://github.com/batleforc/proxyauthK8s/commit/0ebe238367032340204146ce3085ffbd4dc05c4f)) - dependabot[bot]
#### Miscellaneous Chores
- (**pre-release**) Pre-Release commit - ([cddc5ac](https://github.com/batleforc/proxyauthK8s/commit/cddc5acc12b6b041d8f4d7a77d08a63af9b9578c)) - Max Batleforc

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).
