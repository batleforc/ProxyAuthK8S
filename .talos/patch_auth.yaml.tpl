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
            claimValidationRules:
              - expression: "claims.email_verified == true"
                message: "email must be verified"
            claimMappings:
              username:
                expression: '"weebsso:" + claims.email'
              groups:
                expression: "claims.groups"
              uid:
                expression: "claims.sub"
            userValidationRules:
              - expression: "!user.username.startsWith('system:')"
                message: "username cannot used reserved system: prefix"
              - expression: "user.groups.all(group, !group.startsWith('system:'))"
                message: "groups cannot used reserved system: prefix"
      permissions: 0o444
      path: /var/lib/apiserver/authentication.yaml
      op: create
