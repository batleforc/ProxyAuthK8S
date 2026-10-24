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
back's in embedded mode; the headless Service gives each one a DNS name,
`<pod>.<headless>.<namespace>.svc`, which the back composes itself (T19).
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
- name: POD_NAMESPACE
  valueFrom:
    fieldRef:
      fieldPath: metadata.namespace
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
