{{- define "vcsstar.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "vcsstar.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "vcsstar.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "vcsstar.selectorLabels" . }}
app.kubernetes.io/version: {{ .Values.image.tag | default .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "vcsstar.selectorLabels" -}}
app.kubernetes.io/name: {{ include "vcsstar.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/* Secret with the database user's `username` and `password` keys. */}}
{{- define "vcsstar.pgSecret" -}}
{{- if eq .Values.postgresql.operator "cnpg" -}}
{{ include "vcsstar.fullname" . }}-postgres-app
{{- else -}}
{{ .Values.postgresql.user }}.{{ include "vcsstar.fullname" . }}-postgres.credentials.postgresql.acid.zalan.do
{{- end -}}
{{- end }}

{{/* Service that reaches the primary. */}}
{{- define "vcsstar.pgHost" -}}
{{- if eq .Values.postgresql.operator "cnpg" -}}
{{ include "vcsstar.fullname" . }}-postgres-rw
{{- else -}}
{{ include "vcsstar.fullname" . }}-postgres
{{- end -}}
{{- end }}
