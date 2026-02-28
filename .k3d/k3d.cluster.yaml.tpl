apiVersion: k3d.io/v1alpha5
kind: Simple
metadata:
  name: proxy-auth-k8s
servers: 1
agents: 0
hostAliases: # /etc/hosts style entries to be injected into /etc/hosts in the node containers and in the NodeHosts section in CoreDNS
  - ip: TRAEFIK_IP
    hostnames:
      - authelia.k8s.localhost
      - traefik.k8s.localhost
      - k8s.localhost
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
  - description: 'Source: Embedded, Destination: Magic shortcut path'
    source: |
      apiVersion: apiserver.config.k8s.io/v1beta1
      kind: AuthenticationConfiguration
      jwt:
        - issuer:
            url: 'https://authelia.k8s.localhost'
            audiences:
              - 'kube_login'
            audienceMatchPolicy: MatchAny
            certificateAuthority: |
              CERTIFICATE
          claimMappings:
            username:
              expression: '"TestUser"'
            groups:
              expression: "'developers'"
            uid:
              expression: "claims.sub"
          userValidationRules:
            - expression: "user.groups.all(group, !group.startsWith('system:'))"
              message: "groups cannot used reserved system: prefix"
    destination: /var/lib/apiserver/authentication.yaml
