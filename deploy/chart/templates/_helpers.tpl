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
