# Pipeline de IA

## Princípio

Gemini recebe texto e timestamps, nunca o vídeo. FFmpeg executa decisões audiovisuais; Gemini interpreta conteúdo.

## Provider

O módulo `ai` deve esconder HTTP, autenticação, modelo, retries e limites de tokens atrás de um contrato de aplicação, por exemplo `DidacticContentGenerator`. A chave fica somente no backend, em variável de ambiente ou secret do deployment.

Cada solicitação registra:

- hash da transcrição de entrada;
- versão do prompt;
- modelo/configuração;
- schema esperado;
- resposta bruta e resposta validada;
- custo/latência quando disponível.

## Operações

1. capítulos e títulos;
2. identificação de possíveis retakes;
3. resumo executivo;
4. material didático;
5. glossário e questões.

Respostas estruturadas devem ser validadas antes de persistir. Se o JSON vier inválido, fazer uma correção limitada/retry; não aceitar texto livre onde a aplicação espera timestamps.

## Retakes e cortes

Um possível retake vira uma sugestão com evidência textual e confiança. Nunca vira `REMOVE` automaticamente. O usuário decide na timeline.

## Contexto e chunking

Textos longos são divididos em blocos com overlap controlado. Primeiro gerar análises por bloco; depois consolidar em uma etapa curta. O hash do input e da configuração permite reutilizar resultados.

RAG de PDF/PPTX/DOCX fica para depois: primeiro persistir `source_documents` e uma interface de recuperação, sem introduzir vector database antes de haver um caso de uso medido.

