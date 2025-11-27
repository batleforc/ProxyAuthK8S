# 🚀 Release v0.1.0

This pull request prepares the release for version `v0.1.0`.

## 📊 Release Preparation Summary

| Check | Status |
|-------|--------|
| 📈 Version Increment | ✅ Completed |
| 🔍 Code Quality (Backend) | ✅ Passed |
| 🔍 Code Quality (Frontend) | ❌ Failed |
| 📝 Changelog | ✅ Generated |
| 🔒 Security Audit | ❌ Failed |
| ✅ Commit Validation | ❌ Failed |
| 🐳 Docker Build | ❌ Failed |
| 🔍 Docker Security | ❌ Failed |
| 📋 Swagger Validation | ❌ Failed |

## 📋 Detailed Reports

## 🏗️ Built Artifacts

### Docker Images
- 🖥️ Server: ``
- 🌐 Frontend: ``

### 📋 SBOM Reports
- Server SBOM available in artifacts
- Frontend SBOM available in artifacts

## 📝 Changelog

# Changelog for v0.1.0

# 0.1.0 (2025-11-27)


### Bug Fixes

* **ci:** run migration migrate for nx ([e89508b](https://github.com/batleforc/ProxyAuthK8S/commit/e89508b30c262d750e75f1438bb267f39673bfd2))
* **controller:** path used in manifest ([7ec07f9](https://github.com/batleforc/ProxyAuthK8S/commit/7ec07f996525c53c4a6e608d32d123b9a25896b9))
* **devfile:** dbgate mount path ([6df2361](https://github.com/batleforc/ProxyAuthK8S/commit/6df23617fd44192a4a4c3f1c09af4adaa67e6161))
* it should work ([c9067a6](https://github.com/batleforc/ProxyAuthK8S/commit/c9067a63ebf30b712f3283dba46a8d493182daf0))
* **utoipa:** automate the generation of Swagger api ([a4a4ba0](https://github.com/batleforc/ProxyAuthK8S/commit/a4a4ba079244a95c4614548d57014a4a18d2d8bb))
* wip ([96a2ef0](https://github.com/batleforc/ProxyAuthK8S/commit/96a2ef0ce7e88a9a8a6ca187b7e185b057c36908))
* wip ([49a4e95](https://github.com/batleforc/ProxyAuthK8S/commit/49a4e9562bb8a56aa7a342b4c75b81c3c8bbf5b6))


### Features

* add a local talos cluster ([598dee8](https://github.com/batleforc/ProxyAuthK8S/commit/598dee838389e7911d7987f594e35ae39112f8bb))
* add element in crd or api model ([984c44e](https://github.com/batleforc/ProxyAuthK8S/commit/984c44e5bb85dd0ebd6540fce17f113ac3d44c15))
* Add Redis and go back to trace, something is missing ([1ca2559](https://github.com/batleforc/ProxyAuthK8S/commit/1ca2559bc7039ca18d0c295d609b64f2023361bd))
* **api:** Cluster delegated auth is working ([024914c](https://github.com/batleforc/ProxyAuthK8S/commit/024914cfc8c797bdcfd5defcb2a355dc998f557a))
* **api:** create crd to oidcconf ([924b352](https://github.com/batleforc/ProxyAuthK8S/commit/924b3527b694f854bb333d22a1ef7de9fa8f4c48))
* **api:** return the cluster visible by the current user ! ([c0c72aa](https://github.com/batleforc/ProxyAuthK8S/commit/c0c72aa4671018c37709970c397437c36081c62c))
* **api:** Start work on delegated OIDC Handling ([b800a59](https://github.com/batleforc/ProxyAuthK8S/commit/b800a5985e705dfc0e676ab6661b06f1d9fe6ba2))
* **api:** wip ([de5b252](https://github.com/batleforc/ProxyAuthK8S/commit/de5b252a8f8efe90d668cee108cd16a98b7b8fe9))
* **api:** WIP on auth ([f866331](https://github.com/batleforc/ProxyAuthK8S/commit/f866331016ce5f90d402117c4dfa653e4b46ce1c))
* **api:** WIP on the login endpoint, redirect is the next step ([feebe7f](https://github.com/batleforc/ProxyAuthK8S/commit/feebe7f2fa29668a8201e05d8a828997068909d2))
* **api:** Work on the auth validator for each route ([ee570e8](https://github.com/batleforc/ProxyAuthK8S/commit/ee570e832baa4dc03b44b011ebe29666f414972c))
* auth work, next step "mtls" auth ([97c5522](https://github.com/batleforc/ProxyAuthK8S/commit/97c55226a5b253144a4866b9e9c35ef1a207fd1b))
* **auth:** add front authentication ([c7b0ac1](https://github.com/batleforc/ProxyAuthK8S/commit/c7b0ac1bac82990ca0412e8e85c22f4ac75d6bff))
* **chart:** add Otel ([95d8599](https://github.com/batleforc/ProxyAuthK8S/commit/95d859970a20b982d6cc2575518a05da08e77872))
* **chart:** create crds chart ([ede39d8](https://github.com/batleforc/ProxyAuthK8S/commit/ede39d85184420af05315ff82ebf55616c94b5f1))
* **charts:** add ressources to otel ([474019b](https://github.com/batleforc/ProxyAuthK8S/commit/474019be476c3595f62d6714c591eb5b00be0cde))
* **ci/cd:** add dispatch for check lint ([b2b7bf1](https://github.com/batleforc/ProxyAuthK8S/commit/b2b7bf1203aabc5dd97f19439b37c366aae7f6b1))
* **ci/cd:** fix clippy and setup pipeline ([535d194](https://github.com/batleforc/ProxyAuthK8S/commit/535d1946010fb5e6a89877cae1a3f3399f0af195))
* **ci/cd:** new ci to prepare a new release ([806c019](https://github.com/batleforc/ProxyAuthK8S/commit/806c0191fdd2654af7cf891766b2ea6732c22a5b))
* **controller:** add from/to json to ProxyKubeApi ([c3f7f14](https://github.com/batleforc/ProxyAuthK8S/commit/c3f7f14a71175b8bd4ab7bf778b9b232c32b3065))
* **controller:** init controller ([17957cb](https://github.com/batleforc/ProxyAuthK8S/commit/17957cba1e9556ba0ed3ff2aeb049b4b6bd1102e))
* **controller:** make sure the service is reachable before exposing it ([fc3a91f](https://github.com/batleforc/ProxyAuthK8S/commit/fc3a91fc46885da1bbf5d2d581d045d2fdec49dd))
* **controller:** Start créating listener ([46b7bec](https://github.com/batleforc/ProxyAuthK8S/commit/46b7bec0f6ab213a07b8c934e3e6814c972f7aee))
* **crd:** add oidc block ([bf53b78](https://github.com/batleforc/ProxyAuthK8S/commit/bf53b78d9cd47e9c177bffb8acc7be59978fc400))
* **crd:** make the CRD easyer to use ([4aa1bbe](https://github.com/batleforc/ProxyAuthK8S/commit/4aa1bbe06d5929c9152f73c79fb3f097136d484d))
* create basic helm chart ([6e44a25](https://github.com/batleforc/ProxyAuthK8S/commit/6e44a25fe83f86daaf958b9658769c5f90cd005e))
* **devfile:** Add a Devfile ([6d00fbb](https://github.com/batleforc/ProxyAuthK8S/commit/6d00fbb3df87381c25874a0349aed703da72aad7))
* **devfile:** correct dbgate/devfile and change port to my favorite one ([22bde75](https://github.com/batleforc/ProxyAuthK8S/commit/22bde75adc8cfc258a68d1ce9dc8d219b4c1d76e))
* **front-api:** add gen swagger api for front-api ([e34a1ba](https://github.com/batleforc/ProxyAuthK8S/commit/e34a1bad2ac9e6f544f5c718f6afef7248204978))
* **front/back:** Mise en place de l'authentification dédié clusters ([cfdff15](https://github.com/batleforc/ProxyAuthK8S/commit/cfdff15d4e0ac37aece234d5f0067ead1829457d))
* **front:** add annotation and WIP getallvisiblecluster ([fb0d3c9](https://github.com/batleforc/ProxyAuthK8S/commit/fb0d3c97077ce8f439762f0b37704a5e9d40b619))
* **front:** add maz ui has a component provider ([71a380a](https://github.com/batleforc/ProxyAuthK8S/commit/71a380aa3b6754faf4ab547125777af22454cdf4))
* **front:** add missing param to get the userinfo and fix the callback ([8ca8273](https://github.com/batleforc/ProxyAuthK8S/commit/8ca82739a0c34ec8e4cb0b7c5167e9c7ab5e28de))
* **front:** add pinia ([cfded20](https://github.com/batleforc/ProxyAuthK8S/commit/cfded201fdee6b92e3e80d3baf018e76f6b41cbf))
* **front:** create front app ([59055e7](https://github.com/batleforc/ProxyAuthK8S/commit/59055e784f09bb2b2f2dddad64dbca5f545414d4))
* **front:** Setup auth guard and requiresAuth meta descriptor ([7a6e64e](https://github.com/batleforc/ProxyAuthK8S/commit/7a6e64e5aff01e048b724fe5d2ccc151a823725f))
* **front:** store user and create getter ([28b68c5](https://github.com/batleforc/ProxyAuthK8S/commit/28b68c5c9db50ba241c549535ae95e2476618eeb))
* **front:** turn to dark mode ([4a69d21](https://github.com/batleforc/ProxyAuthK8S/commit/4a69d212874b72e751c45a795063876033ff4bc1))
* **front:** wip CallbackView ([203ac14](https://github.com/batleforc/ProxyAuthK8S/commit/203ac148da866bdaf7eb195b73131446624520cc))
* **front:** WIP on the nav bar ([94006f3](https://github.com/batleforc/ProxyAuthK8S/commit/94006f3002a2d57decddd03c4cbc7c2b4b70ee90))
* **front:** working navbar ([72ba06d](https://github.com/batleforc/ProxyAuthK8S/commit/72ba06d08894692958bde04842dcdbddf4df8fcb))
* init base work of redirect ([8c4af60](https://github.com/batleforc/ProxyAuthK8S/commit/8c4af609cd51161d9c06f34e61f6cadb71081e14))
* Init ProxyAuthK8s project ([77971ab](https://github.com/batleforc/ProxyAuthK8S/commit/77971abf9a410d095a3bbdf5204844ebf851c693))
* Initial commit ([7e79a7c](https://github.com/batleforc/ProxyAuthK8S/commit/7e79a7cc31943ac20adc0a58a6b20659c83f1efa))
* make it build and work on having a proper compose to try it all ([a43760d](https://github.com/batleforc/ProxyAuthK8S/commit/a43760dbec77e4110799e2b6a2257e8eda409527))
* make redis secret or env ([95633ff](https://github.com/batleforc/ProxyAuthK8S/commit/95633ffffed608376409251a7d3418f20c1d80ea))
* Mise en place communication authentifier entre le front/back ([c2a619c](https://github.com/batleforc/ProxyAuthK8S/commit/c2a619cc7df0fe6e99cb09ddf6ddfc1b044437c7))
* **readme:** add more info related on the project ([b0468d7](https://github.com/batleforc/ProxyAuthK8S/commit/b0468d75cdc884d3257deddac7b5a8753d31771d))
* **redirect:** add more trace like the exit code ([ed179ec](https://github.com/batleforc/ProxyAuthK8S/commit/ed179ecd3d131adc3ee22e59b145d12e1d03e1f6))
* **redis:** move from r2d2 to Deadpool, would be a headack perf wise ([c876e98](https://github.com/batleforc/ProxyAuthK8S/commit/c876e9864b880e63cf024f2678479a5f0130f7d7))
* send reqwest to the upstream server ([1533924](https://github.com/batleforc/ProxyAuthK8S/commit/153392455b4b1d5c5ca7dd14475a554131bd55b1))
* **state:** prepare global state ([2553ec6](https://github.com/batleforc/ProxyAuthK8S/commit/2553ec6a3442f103f9079725061713ef64bbb196))
* **swagger:** dont create front-api for route that should not be called ([56e5341](https://github.com/batleforc/ProxyAuthK8S/commit/56e53411efb5f34b6c6500613293e2c1a8ec89db))
* **taskfile:** add lab taskfile ([2ab4183](https://github.com/batleforc/ProxyAuthK8S/commit/2ab4183295a3bc14bf597b80c057c0ea5bdc6047))
* **tracing:** IT WORKS ([8b138e0](https://github.com/batleforc/ProxyAuthK8S/commit/8b138e0b2382e5281d06e4792f0789283543693a))
* **tracing:** WIP ATM it does not work ([56536b8](https://github.com/batleforc/ProxyAuthK8S/commit/56536b8abdaf311bd3ec07f01f507fe20f36fe7a))
* **tracing:** WIP on tracing ([97d883e](https://github.com/batleforc/ProxyAuthK8S/commit/97d883e99515b91bb0530278b473cab2d08e52f3))
* **vscode:** add taskfile extensions ([9d92bb3](https://github.com/batleforc/ProxyAuthK8S/commit/9d92bb337d188eec98a4d9f88c10f5ba11ce0145))
* wip ([f9ea72c](https://github.com/batleforc/ProxyAuthK8S/commit/f9ea72c3d2303c4048c6496917f8177327e542ed))
* wip ([b81014f](https://github.com/batleforc/ProxyAuthK8S/commit/b81014facb6fe605fe089d7a3db1a6420a2e92ad))
* wip ([8b3a7c9](https://github.com/batleforc/ProxyAuthK8S/commit/8b3a7c9c0a13642f19426e496785f8945589ad66))
* wip ([82b21dd](https://github.com/batleforc/ProxyAuthK8S/commit/82b21dd9ee6bc87bc34b05daa80434c6d1d6ce04))
* wip ([0563580](https://github.com/batleforc/ProxyAuthK8S/commit/056358045f831d3ad138c9eb5b3a0541b9d0d137))
* wip ([9f0803c](https://github.com/batleforc/ProxyAuthK8S/commit/9f0803ce24ae0c6f05bd371cb78432731929050d))
* wip ([2e26ebc](https://github.com/batleforc/ProxyAuthK8S/commit/2e26ebcff7c63314c6eeb7279d3690515aaedea5))
* WIP add authelia support ([13e4321](https://github.com/batleforc/ProxyAuthK8S/commit/13e4321214811a31025425dc3772cccb3243dd4a))
* WIP HTTPs ([2b2a3d2](https://github.com/batleforc/ProxyAuthK8S/commit/2b2a3d2709f06c8f18ecffc4b2e427afc59a53d4))
* WIP on bringing OIDC to the talos cluster ([f52e957](https://github.com/batleforc/ProxyAuthK8S/commit/f52e957ac1d74c6dd25373189592525db39c99b6))
* wip on tracing and OIDC ([5b39d84](https://github.com/batleforc/ProxyAuthK8S/commit/5b39d8422ce094a5a208e7f900a4d2e5a3505aab))



