cluster:
  apiServer:
    extraArgs:
      authentication-config: /var/lib/apiserver/authentication.yaml
    extraVolumes:
      - hostPath: /var/lib/apiserver
        mountPath: /var/lib/apiserver
        readonly: true

machine:
  files:
    - content: |
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
      permissions: 0o444
      path: /var/lib/apiserver/authentication.yaml
      op: create
