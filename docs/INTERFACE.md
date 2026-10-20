# Interface e manipulação de mídia

O VideoLab abre em um estado neutro, sem criar projetos automaticamente. O usuário pode criar, abrir e renomear projetos. Projetos já existentes mantêm seus nomes e arquivos, inclusive os criados pela versão demonstrativa.

## Funções disponíveis

| Controle | Comportamento real |
| --- | --- |
| Importar | Envia vídeos/áudios e acompanha inspeção técnica. |
| Buscar / Tudo / Vídeo / Áudio | Filtra o bin; Áudio mostra arquivos sem faixa de vídeo. |
| Seleção | Atualiza monitor e propriedades com a mídia selecionada. |
| Reprodução | Play/pause, avanço/retorno de 5 s e busca temporal. Depende dos codecs suportados pelo navegador. |
| Mover antes/depois | Salva a ordem dos arquivos no projeto e usa essa ordem na exportação. |
| Sequência | Mostra os arquivos proporcionais às suas durações, cursor, zoom e regiões de corte. Clique para selecionar e buscar um ponto no original. |
| Analisar | Detecta silêncio, tela preta, congelamento e volume baixo. Clique na sugestão para ouvir/ver o trecho. |
| Cortar | Marca sugestões temporais ou intervalos manuais para remoção. Volume baixo é tratado pela normalização, não como corte. |
| Desfazer/refazer | Reverte a revisão de cortes (até 50 etapas); não reverte importações, reordenação ou renomeação. |
| Normalizar | Liga/desliga a normalização de áudio na exportação. |
| Exportar | Aplica cortes, preserva originais e oferece download das exportações concluídas. |

Os cortes ficam salvos por projeto no armazenamento deste navegador. Ao reabrir o mesmo projeto, são recuperados. O histórico de desfazer/refazer vale para a sessão; a revisão não é sincronizada entre navegadores. O monitor reproduz o original, não simula os cortes ao vivo. Intervalos sobrepostos contam uma única vez na duração estimada.

## Avaliação das funções que eram apenas visuais

Transformação (posição, escala e rotação), efeitos, pistas independentes, mute/solo por pista, encaixe magnético, transições, capítulos e formas de onda foram retirados da interface por não terem execução real no motor atual. O caminho para implementá-los é persistir uma timeline com clipes/pistas, gerar um grafo de filtros FFmpeg e usar o mesmo modelo para a prévia. Formas de onda precisam de dados de áudio extraídos, em vez de ilustrações estáticas.

Prioridade sugerida para a próxima etapa: (1) padronizar resoluções, taxas de quadros e áudio entre fontes para cortes em múltiplos arquivos; (2) persistir a timeline e pré-visualizar o resultado editado; (3) acrescentar transformações e mixagem de pistas com testes de exportação.

Limites atuais do render: exige mídias inspecionadas com vídeo e áudio; cortes funcionam com um arquivo por projeto. Vários arquivos podem ser concatenados sem cortes e devem ter características compatíveis. Essas restrições são mostradas na interface e entradas inválidas são rejeitadas pela API antes de criar o job.

## Verificação

```bash
cd frontend
npm test
npm run build
```

```bash
mise exec -- cargo test --manifest-path backend/Cargo.toml
mise exec -- cargo build --manifest-path backend/Cargo.toml
python3 backend/smoke_test.py
```

O smoke test inicia um backend isolado com banco temporário e mídia sintética. Verifica upload maior que 2 MB, streaming HTTP Range, isolamento entre projetos, análise concluída, cortes inválidos, exportação com duração esperada, normalização, download, ordem e concatenação.
