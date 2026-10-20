# FFmpeg e FFprobe

## Contrato de execução

Usar `tokio::process::Command` com argumentos separados:

```rust
Command::new("ffmpeg")
    .arg("-i")
    .arg(input_path)
    .arg("-progress")
    .arg("pipe:1")
    .arg(output_path)
```

Nunca interpolar input do usuário em `sh -c`. O binário pode ser configurável, mas o caminho deve ser validado no startup e registrado no diagnóstico.

## Probe

Comando-base:

```text
ffprobe -v error -print_format json -show_format -show_streams <input>
```

O parser deve rejeitar JSON sem stream de vídeo ou áudio quando o job exigir esse stream. O JSON original é guardado para auditoria.

## Análises iniciais

```text
silencedetect=noise=-35dB:d=2
blackdetect=d=1:pix_th=0.10
freezedetect=n=-60dB:d=2
volumedetect
astats
ebur128
```

Os parâmetros são configuração versionada, não constantes espalhadas pelo código. A análise não modifica o arquivo original.

Na implementação atual, as ocorrências são persistidas em `analysis_results` com intervalo, confiança, evidência e versão da regra. O endpoint de render só remove ocorrências explicitamente enviadas em `remove_analysis_ids`; nenhuma sugestão é aplicada automaticamente.

## Áudio

O caminho padrão é análise → proposta → loudnorm em dois passos:

1. primeira passagem mede loudness e guarda o JSON do filtro;
2. segunda passagem aplica os valores medidos para o alvo configurado, inicialmente `-16 LUFS`.

Na Fase 2, a primeira passagem é executada sobre o master bruto e a segunda aplica os valores medidos ao áudio final. O resultado é validado novamente com FFprobe antes de marcar o job como concluído.

Filtros como `highpass`, `afftdn`, `acompressor` e `alimiter` devem ser opt-in. Tratamento agressivo não deve ser aplicado apenas porque o áudio está baixo.

## Concatenação

Usar concat demuxer quando os streams forem compatíveis. Quando não forem, normalizar cada input para um perfil canônico antes de concatenar. A decisão fica registrada no RenderPlan.

Não prometer stream copy para cortes arbitrários: cortes precisos frequentemente exigem reencode por causa de keyframes. A economia de CPU é secundária à correção do resultado revisado.

## Progresso e cancelamento

Ler stdout do modo `-progress pipe:1`, interpretando pares `key=value`. O stderr vai para log estruturado. Ao cancelar, enviar término ao processo, aguardar, e validar que nenhum output parcial foi promovido.

## Validação pós-processamento

Todo output deve:

1. terminar com exit code zero;
2. existir e possuir tamanho plausível;
3. passar novamente pelo FFprobe;
4. conter streams esperados;
5. ser promovido atomicamente do temporário para o destino.
