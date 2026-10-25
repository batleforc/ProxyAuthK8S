{{/*
Shared naming and security helpers.
*/}}

{{- define "proxyauthk8s.back.name" -}}
{{ .Release.Name }}-proxyauthk8s-back
{{- end -}}

{{- define "proxyauthk8s.front.name" -}}
{{ .Release.Name }}-proxyauthk8s-front
{{- end -}}

{{- define "proxyauthk8s.otel.name" -}}
{{ .Release.Name }}-otel-collector
{{- end -}}

{{/*
Pod-level security context, shared by every component.
*/}}
{{- define "proxyauthk8s.podSecurityContext" -}}
{{- with .Values.securityContext.pod }}
securityContext:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end -}}

{{/*
Container-level security context, shared by every component.
*/}}
{{- define "proxyauthk8s.containerSecurityContext" -}}
{{- with .Values.securityContext.container }}
securityContext:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end -}}

{{/*
The Redis URL, from values or from the secret the operator provides.

Fails rendering when neither is set: a chart must never ship a default
credential, and silently starting with no Redis is worse than not installing.
*/}}
{{- define "proxyauthk8s.redisUrl" -}}
{{- if eq .Values.back.redis.source "env" -}}
{{- required "back.redis.value.url is required when back.redis.source is \"env\". Set it, or switch to back.redis.source=\"secret\" and provide back.redis.secretName." .Values.back.redis.value.url -}}
{{- end -}}
{{- end -}}

{{/*
The public base URL of the dashboard and API: ingress.publicUrl, else the
scheme and ingress.host. Set publicUrl when clients reach the ingress on a
non-default port or through another proxy.
*/}}
{{- define "proxyauthk8s.publicUrl" -}}
{{- .Values.ingress.publicUrl | default (printf "%s://%s" (ternary "https" "http" .Values.ingress.tls.enabled) .Values.ingress.host) | trimSuffix "/" -}}
{{- end -}}

{{/*
The agent gateway (RFC 0012). Its pods are the gateway Deployment's, or the
back's in embedded mode; the back finds the pod a presence names among the
EndpointSlices of the headless Service (T19).
*/}}
{{- define "proxyauthk8s.gateway.name" -}}
{{ .Release.Name }}-proxyauthk8s-gateway
{{- end -}}

{{- define "proxyauthk8s.gateway.headless" -}}
{{ include "proxyauthk8s.gateway.name" . }}-pods
{{- end -}}

{{- define "proxyauthk8s.gateway.podApp" -}}
{{- if eq .Values.agentGateway.mode "embedded" -}}
{{ include "proxyauthk8s.back.name" . }}
{{- else -}}
{{ include "proxyauthk8s.gateway.name" . }}
{{- end -}}
{{- end -}}

{{- define "proxyauthk8s.gateway.tokenSecret" -}}
{{ .Values.agentGateway.tokenSecretName | default (printf "%s-token" (include "proxyauthk8s.gateway.name" .)) }}
{{- end -}}

{{/*
The gateway settings, for the gateway and for the back that CONNECTs to it.
*/}}
{{- define "proxyauthk8s.gateway.env" -}}
{{- with .Values.agentGateway }}
- name: AGENT_GATEWAY_SERVICE
  value: "{{ include "proxyauthk8s.gateway.headless" $ }}.{{ $.Release.Namespace }}.svc"
- name: AGENT_GATEWAY_TOKEN
  valueFrom:
    secretKeyRef:
      name: "{{ include "proxyauthk8s.gateway.tokenSecret" $ }}"
      key: token
- name: AGENT_GATEWAY_PORT
  value: "{{ .port }}"
- name: AGENT_GATEWAY_CONNECT_PORT
  value: "{{ .connectPort }}"
- name: AGENT_GATEWAY_TUNNELS_PER_IDENTITY
  value: "{{ .limits.tunnelsPerAgentIdentity }}"
- name: AGENT_GATEWAY_STREAMS_PER_TUNNEL
  value: "{{ .limits.streamsPerTunnel }}"
- name: AGENT_GATEWAY_MAX_STREAMS_PER_TUNNEL
  value: "{{ .limits.maxStreamsPerTunnel }}"
- name: AGENT_GATEWAY_HANDSHAKE_TIMEOUT
  value: "{{ .limits.handshakeTimeoutSeconds }}"
- name: AGENT_GATEWAY_ENROLL_PER_MINUTE
  value: "{{ .limits.enrollPerMinute }}"
- name: AGENT_GATEWAY_ENROLL_PER_MINUTE_TOTAL
  value: "{{ .limits.enrollPerMinuteTotal }}"
- name: AGENT_GATEWAY_TLS_MODE
  value: "{{ .tlsMode }}"
- name: POD_NAMESPACE
  valueFrom:
    fieldRef:
      fieldPath: metadata.namespace
{{- end }}
{{- end -}}

{{/*
The passthrough certificate of the pods holding the gateway, at the paths
AGENT_GATEWAY_TLS_CERT_PATH / _KEY_PATH default to.
*/}}
{{- define "proxyauthk8s.gateway.tlsVolumeMount" -}}
{{- if eq .Values.agentGateway.tlsMode "passthrough" }}
- name: agent-tls
  mountPath: /agent-tls
  readOnly: true
{{- end }}
{{- end -}}

{{- define "proxyauthk8s.gateway.tlsVolume" -}}
{{- if eq .Values.agentGateway.tlsMode "passthrough" }}
- name: agent-tls
  secret:
    secretName: {{ required "agentGateway.tls.secretName is required with tlsMode: passthrough" .Values.agentGateway.tls.secretName | quote }}
{{- end }}
{{- end -}}

{{/*
Ingress rules of the pods holding the gateway: the agents' port from the
exposure layer, the CONNECT port from the back's pods only (T18).
*/}}
{{- define "proxyauthk8s.gateway.netpolIngress" -}}
- {{- if .Values.networkPolicy.ingressFrom }}
  from:
    {{- toYaml .Values.networkPolicy.ingressFrom | nindent 4 }}
  {{- end }}
  ports:
    - port: {{ .Values.agentGateway.port }}
      protocol: TCP
- from:
    - podSelector:
        matchLabels:
          app: "{{ include "proxyauthk8s.back.name" . }}"
  ports:
    - port: {{ .Values.agentGateway.connectPort }}
      protocol: TCP
{{- end -}}

{{/*
A PodDisruptionBudget's budget: maxUnavailable, when set, wins over the
minAvailable default (the API refuses both); an explicit 0 is kept, an
empty value is unset. With a single pod at the fewest (`min`,
proxyauthk8s.minReplicas), maxUnavailable: 1. Args: pdb, min.
*/}}
{{- define "proxyauthk8s.pdbBudget" -}}
{{- if not (has (toString .pdb.maxUnavailable) (list "" "<nil>")) -}}
maxUnavailable: {{ .pdb.maxUnavailable }}
{{- else if le (int .min) 1 -}}
{{- /* A single pod: minAvailable would block every drain. */ -}}
maxUnavailable: 1
{{- else if not (has (toString .pdb.minAvailable) (list "" "<nil>")) -}}
minAvailable: {{ .pdb.minAvailable }}
{{- end -}}
{{- end -}}

{{/*
The fewest pods a Deployment runs: autoscaling.minReplicas with its HPA,
else its replica count. Args: replicas, autoscaling.
*/}}
{{- define "proxyauthk8s.minReplicas" -}}
{{- ternary .autoscaling.minReplicas .replicas .autoscaling.enabled | int -}}
{{- end -}}

{{/*
A Deployment's `replicas`: the chart's count, or with an HPA the live one,
so an upgrade neither resets the HPA's choice nor drops the field (Helm's
merge would then fall back to 1 pod); minReplicas when there is none to read
(first install, `helm template`, Argo CD). Args: root, name, replicas,
autoscaling.
*/}}
{{- define "proxyauthk8s.replicas" -}}
{{- if not .autoscaling.enabled -}}
replicas: {{ .replicas }}
{{- else -}}
{{- with (lookup "apps/v1" "Deployment" .root.Release.Namespace .name) -}}
replicas: {{ .spec.replicas }}
{{- else -}}
replicas: {{ .autoscaling.minReplicas }}
{{- end -}}
{{- end -}}
{{- end -}}

{{/*
The scale-down of an HPA whose pods hold agents' tunnels (the gateway, or
the back in embedded mode): each removal closes tunnels, so slowly.
*/}}
{{- define "proxyauthk8s.tunnelScaleDown" -}}
behavior:
  scaleDown:
    stabilizationWindowSeconds: {{ .scaleDownStabilizationSeconds }}
    policies:
      - type: Pods
        value: {{ .scaleDownPods }}
        periodSeconds: {{ .scaleDownPeriodSeconds }}
{{- end -}}
