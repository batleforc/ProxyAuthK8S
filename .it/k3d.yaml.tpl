# Integration bench cluster (RFC 0009 §6.1), derived from
# .k3d/k3d.cluster.yaml.tpl. `it:up` fills __IMAGE__ and __CA__.
apiVersion: k3d.io/v1alpha5
kind: Simple
metadata:
  name: proxyauth-it
servers: 1
agents: 0
image: __IMAGE__
ports:
  # 127.0.0.1 only: the bench carries known passwords.
  - port: 127.0.0.1:8443:8443
    nodeFilters:
      - loadbalancer
# These names resolve to Traefik's pinned ClusterIP (HelmChartConfig below):
# in the node's /etc/hosts for the apiserver, in CoreDNS for the pods.
hostAliases:
  - ip: 10.43.0.200
    hostnames:
      - proxyauth.k8s.localhost
      - dex.k8s.localhost
      # RFC 0010 §6.P: the cluster's IdP of the TokenExchange clusters.
      - dex-b.k8s.localhost
      # RFC 0012: the agents dial the gateway through the Ingress.
      - agent.k8s.localhost
# Traefik's HelmChartConfig (websecure on 8443, ClusterIP 10.43.0.200), from
# .it/traefik-config.yaml. A bind mount: k3s' data directory is a volume, so a
# file k3d writes under it is hidden.
volumes:
  - volume: __TRAEFIK_CONFIG__:/var/lib/rancher/k3s/server/manifests/traefik-config.yaml
    nodeFilters:
      - "server:*"
  # RFC 0014: the requestheader CA file, a directory so `it:up` can swap the
  # file in place once k3s generated its own aggregation CA (decision 58).
  - volume: __FRONT_PROXY_DIR__:/var/lib/front-proxy
    nodeFilters:
      - "server:*"
options:
  kubeconfig:
    updateDefaultKubeconfig: false
    switchCurrentContext: false
  k3s:
    extraArgs:
      - arg: "--kube-apiserver-arg=authentication-config=/var/lib/apiserver/authentication.yaml"
        nodeFilters:
          - "server:*"
      # RFC 0014 §6.6: a dedicated front-proxy CA next to k3s' own, and the
      # CN of the proxy's certificate next to k3s' aggregator.
      - arg: "--kube-apiserver-arg=requestheader-client-ca-file=/var/lib/front-proxy/requestheader-ca.crt"
        nodeFilters:
          - "server:*"
      - arg: "--kube-apiserver-arg=requestheader-allowed-names=system:auth-proxy,proxyauth-it"
        nodeFilters:
          - "server:*"
      - arg: "--kubelet-arg=feature-gates=KubeletInUserNamespace=true"
        nodeFilters:
          - "server:*"
files:
  - source: |
      apiVersion: apiserver.config.k8s.io/v1beta1
      kind: AuthenticationConfiguration
      jwt:
        - issuer:
            url: https://dex.k8s.localhost:8443
            audiences:
              - kube_login
              - kube_oauth
            # Required by the apiserver as soon as there are two audiences.
            audienceMatchPolicy: MatchAny
            certificateAuthority: |
      __CA__
          # The apiserver takes `email` as the username only once verified.
          claimValidationRules:
            - expression: "claims.email_verified == true"
              message: "email must be verified"
          # The real claims, so a test sees the user Dex asserted.
          claimMappings:
            username:
              expression: "claims.email"
            groups:
              expression: "has(claims.groups) ? claims.groups : []"
            uid:
              expression: "claims.sub"
          userValidationRules:
            - expression: "user.groups.all(group, !group.startsWith('system:'))"
              message: "groups cannot use the reserved system: prefix"
        # RFC 0010 §6.P: Dex B, the IdP of the TokenExchange clusters. Dex B
        # re-encodes `sub` (decision 11): the tests' RBAC names the groups.
        - issuer:
            url: https://dex-b.k8s.localhost:8443
            audiences:
              - kube-prod
            certificateAuthority: |
      __CA__
          claimMappings:
            username:
              expression: '"dexb:" + claims.sub'
            groups:
              expression: 'has(claims.groups) ? claims.groups.map(g, "dexb:" + g) : []'
        # RFC 0014 §6.P point 10: the proxy as the issuer of a test cluster; the
        # apiserver checks the prefix itself (decision 28).
        - issuer:
            url: https://proxyauth.k8s.localhost:8443/issuer/it-rfc0014-p10/c
            audiences:
              - proxyk8sauth:it-rfc0014-p10/c
            certificateAuthority: |
      __CA__
          claimMappings:
            username:
              claim: sub
              prefix: ""
            groups:
              claim: groups
              prefix: ""
          userValidationRules:
            - expression: "user.username.startsWith('proxyauth:')"
              message: "the proxy asserts prefixed users only"
            - expression: "user.groups.all(g, g.startsWith('proxyauth:'))"
              message: "the proxy asserts prefixed groups only"
        # Point 12: the same without userValidationRules, which the probe must
        # see (IssuerNotTrusted).
        - issuer:
            url: https://proxyauth.k8s.localhost:8443/issuer/it-rfc0014-p12/c
            audiences:
              - proxyk8sauth:it-rfc0014-p12/c
            certificateAuthority: |
      __CA__
          claimMappings:
            username:
              claim: sub
              prefix: ""
            groups:
              claim: groups
              prefix: ""
        # Point 13: rotation and revocation.
        - issuer:
            url: https://proxyauth.k8s.localhost:8443/issuer/it-rfc0014-p13/c
            audiences:
              - proxyk8sauth:it-rfc0014-p13/c
            certificateAuthority: |
      __CA__
          claimMappings:
            username:
              claim: sub
              prefix: ""
            groups:
              claim: groups
              prefix: ""
          userValidationRules:
            - expression: "user.username.startsWith('proxyauth:')"
              message: "the proxy asserts prefixed users only"
            - expression: "user.groups.all(g, g.startsWith('proxyauth:'))"
              message: "the proxy asserts prefixed groups only"
    destination: /var/lib/apiserver/authentication.yaml
