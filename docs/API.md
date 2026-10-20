# API inicial

## Editor web: endpoints implementados

Todos usam o prefixo `/api`:

```text
PATCH /projects/:id                              # {"name":"Meu projeto"}
PATCH /projects/:id/media/order                  # {"media_ids":["id-1","id-2"]}
GET   /projects/:id/media/:media_id/content       # streaming com HTTP Range
GET   /projects/:id/renders                      # lista de exportações e status
GET   /projects/:id/render-plans/:plan_id/download # somente exportações concluídas
```

O `POST /projects/:id/render` aceita `manual_cuts: [{media_id, start_ms, end_ms}]`, além de `remove_analysis_ids` e `normalize_audio`. Intervalos devem estar dentro da mídia e pertencer ao projeto. O render rejeita cortes que eliminam todo o conteúdo, sugestões inválidas, fontes sem vídeo/áudio e cortes em múltiplos arquivos (limite atual do motor).

As seções abaixo descrevem também endpoints planejados: transcrição, material, cancelamento e edição de segmentos ainda não estão implementados.

API REST versionada implicitamente em `/api`. Operações longas apenas criam jobs.

## Projetos e mídia

```text
POST  /projects
GET   /projects
GET   /projects/:project_id
POST  /projects/:project_id/media
GET   /projects/:project_id/media
PATCH /projects/:project_id/media/order
```

## Pipeline

```text
POST /projects/:project_id/analyze
POST /projects/:project_id/transcribe
POST /projects/:project_id/process
POST /projects/:project_id/render
POST /projects/:project_id/material
GET  /projects/:project_id/timeline
PATCH /segments/:segment_id
```

## Jobs e eventos

```text
GET    /jobs/:job_id
POST   /jobs/:job_id/cancel
GET    /jobs/:job_id/events   # text/event-stream
```

Exemplo de criação de job:

```json
{
  "job_id": "uuid",
  "type": "ANALYZE_AUDIO",
  "status": "QUEUED"
}
```

Exemplo de evento SSE:

```text
event: progress
data: {"job_id":"uuid","progress":0.73,"step":"loudnorm"}
```

## Regras

- upload usa multipart e possui limite configurável;
- `POST` de processamento é idempotente por configuração/input quando possível;
- `PATCH` de segmento aceita apenas transições válidas;
- paths de storage nunca são aceitos diretamente do cliente;
- erros retornam código estável e `request_id`.
