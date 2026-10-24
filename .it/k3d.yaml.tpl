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
      # RFC 0012: the agents dial the gateway through the Ingress.
      - agent.k8s.localhost
# Traefik's HelmChartConfig (websecure on 8443, ClusterIP 10.43.0.200), from
# .it/traefik-config.yaml. A bind mount: k3s' data directory is a volume, so a
# file k3d writes under it is hidden.
volumes:
  - volume: __TRAEFIK_CONFIG__:/var/lib/rancher/k3s/server/manifests/traefik-config.yaml
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
    destination: /var/lib/apiserver/authentication.yaml
