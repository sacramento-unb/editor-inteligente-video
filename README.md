# Editor Inteligente de Vídeo

Pipeline local e não destrutivo para transformar gravações de aulas em vídeos revisados, episódios, legendas e material didático.

O backend já cobre as fases 1 e 2. O frontend é uma interface inspirada em estações de edição como o Kdenlive, conectada à API para carregar mídias, analisar sugestões, revisar cortes e disparar renderizações.

## Decisão central

```text
Rust = orquestração, API, persistência e jobs
FFprobe = inspeção técnica
FFmpeg = análise audiovisual e renderização
Whisper local = transcrição
Gemini = interpretação textual e material didático
```

O arquivo original é imutável. A análise produz dados; a revisão produz um plano de edição; somente o plano aprovado produz arquivos de saída.

Na implementação atual, Rust, SQLite, FFprobe e FFmpeg cobrem ingestão, análise
determinística, revisão e renderização. Whisper e Gemini são arquitetura
planejada: não existem módulos, dependências ou endpoints ativos para esses dois
providers neste commit.

## Documentação

- [Arquitetura](docs/ARCHITECTURE.md)
- [Pipeline de mídia](docs/MEDIA_PIPELINE.md)
- [Integração com FFmpeg/FFprobe](docs/FFMPEG.md)
- [Transcrição](docs/TRANSCRIPTION.md)
- [Pipeline de IA](docs/AI_PIPELINE.md)
- [API](docs/API.md)
- [Banco de dados](docs/DATABASE.md)
- [Interface: controles, manipulação e limitações](docs/INTERFACE.md)

## Execução local

As operações abaixo já estão implementadas na Fase 1:

```bash
mise exec -- cargo run --manifest-path backend/Cargo.toml
```

```text
GET  /health
POST /api/projects
GET  /api/projects
GET  /api/projects/:id
POST /api/projects/:id/media       # multipart: file
GET  /api/projects/:id/media
POST /api/projects/:id/analyze
GET  /api/jobs/:id
GET  /api/jobs/:id/events          # SSE
```

Configuração opcional: `DATABASE_URL`, `STORAGE_ROOT`, `FFPROBE_BIN`, `MAX_UPLOAD_BYTES` e `BIND_ADDR`.

## Interface web conteinerizada

A forma recomendada de executar o projeto completo é via Docker Compose:

```bash
docker compose up -d --build
```

Abra [http://127.0.0.1:8080](http://127.0.0.1:8080). A porta pode ser alterada com `VIDEO_EDITOR_PORT`, por exemplo:

```bash
VIDEO_EDITOR_PORT=8090 docker compose up -d
```

Os arquivos enviados, o banco SQLite e os renders ficam no volume Docker `video_editor_data`. Para acompanhar os serviços ou encerrá-los:

```bash
docker compose logs -f
docker compose down
```

O Nginx serve a interface e encaminha `/api` e `/health` para o backend. O backend inclui FFmpeg e FFprobe na própria imagem.

## Interface local de desenvolvimento

Para abrir o mockup visual:

```bash
cd frontend
npm install
npm run dev
```

O layout fica disponível no endereço exibido pelo Vite. Para usar uma API local em outra porta, defina `VITE_BACKEND_URL`.

## Fase 2 implementada

Também estão disponíveis:

```text
POST /api/projects/:id/analyze
GET  /api/projects/:id/timeline
POST /api/projects/:id/render
GET  /api/projects/:id/render-plans/:plan_id
```

O render aceita `remove_analysis_ids` e `normalize_audio`. Os originais permanecem intactos.

## Verificação segura

Os testes não exigem iniciar o servidor nem processar mídia real:

```bash
cargo test --locked --manifest-path backend/Cargo.toml
cd frontend
npm ci
npm test
npm run build
```

Esses comandos cobrem parsers, sanitização de nomes, combinação de cortes e
lógica de edição no frontend. Upload real, SSE, FFmpeg/FFprobe, persistência,
Docker e renderização ponta a ponta continuam dependentes de validação de
integração.

Não há arquivo de licença no nível do repositório. Confirme os termos antes de
reutilizar ou redistribuir o projeto.
