machine:
  network:
    extraHostEntries:
      - ip: TRAEFIK_IP
        aliases:
          - authelia.k8s.localhost
          - traefik.k8s.localhost
          - k8s.localhost