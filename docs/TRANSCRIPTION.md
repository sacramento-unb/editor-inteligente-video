# Transcrição

## Contrato

O domínio depende de uma abstração:

```rust
trait TranscriptionProvider {
    async fn transcribe(&self, input: &Path, options: TranscriptionOptions)
        -> Result<Transcript>;
}
```

O formato interno é independente do provider:

```json
{
  "language": "pt",
  "segments": [
    {
      "start_ms": 12400,
      "end_ms": 18700,
      "text": "Hoje vamos falar sobre banco de dados espacial."
    }
  ]
}
```

## Provider inicial

Usar Whisper local em processo separado, preferencialmente faster-whisper quando disponível. Rust continua dono do job, timeout, armazenamento e normalização; o provider só recebe um arquivo e devolve segmentos.

O adaptador pode trocar a implementação por Whisper.cpp, provider remoto ou outro modelo sem alterar `transcription_segments`.

## Ordem no MVP

Transcrever o master editado uma vez. Ao gerar episódios, filtrar segmentos pelo intervalo da parte e subtrair o offset para gerar SRT/VTT local. Isso evita uma inferência repetida por episódio.

## Falhas e custo

Modelo, idioma, dispositivo e tamanho de lote ficam em `TranscriptionOptions` e no job. Se o provider falhar, os vídeos continuam disponíveis; apenas os artefatos de transcrição permanecem `FAILED`.

