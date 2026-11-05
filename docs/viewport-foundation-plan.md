# Plano de implementação — base da viewport multimídia

Escopo: executar a fundação de F015/F021 neste projeto independente. Não migrar código,
assets privados ou funcionalidades do editor original. Contrato de referência:
[F021](features/viewport.md) e [arquitetura](core-architecture.md).

## Fatias e critérios

1. **Modelo público:** projeto vazio/configurável, IDs próprios, assets tipados, tracks/clipes
   criados por comandos e fixtures demo separadas. Preservar o workstation atual e seus testes.
2. **Avaliação:** tempo racional/configurável, preview de fonte ou sequência, resolução de
   camadas/tempos/câmera num plano independente de GPUI. Operações por IDs, sem seleção obrigatória.
3. **Execução:** interfaces de backend de render/áudio e capacidades, requests/revisões,
   carregamento/erro/cancelamento e descarte de respostas antigas. Bridge versionada Rust↔JS.
4. **Three.js:** runtime local com cena procedural pequena e backend splat explicitamente
   escolhido/versionado; carregar, configurar câmera, seek e render com confirmação. Sem CDN.
5. **Headless:** biblioteca e CLI orquestram o mesmo runtime/plano para salvar frames sem janela
   do editor. Comparar frames em tempos conhecidos; não substituir por screenshot desktop.
6. **Desktop:** host de viewport persistente no monitor único, seleção fonte/sequência, estados
   de carregamento/erro e transporte compartilhado. Avaliar `gpui-fast` + WebView numa POC antes
   de adotar backend; validar overlays, foco, resize, ocultação e liberação de recursos.
7. **Multimídia:** fixtures pequenas/determinísticas de imagem, vídeo, áudio e texto para provar
   alternância e composição. Declarar formatos iniciais, unidades e limitações; waveform não é
   prova de saída de áudio, alterar estado não é prova de render.
8. **Contrato e evidências:** atualizar docs por feature/matriz/skill quando necessário,
   registrar APIs e comandos reais e produzir relatório com provas e gaps por superfície.

## Verificação

Fluxo de aceitação: criar projeto → cadastrar fontes → criar tracks/clipes → configurar
câmera → avaliar sequência em tempos conhecidos → renderizar frame pela lib/CLI sem desktop →
mostrar o mesmo pedido no monitor. Exercitar conteúdo misto, troca de fonte, cancelamento,
recurso inválido, callback antigo e ausência de backend. Verificar amostras de áudio/saída
quando implementadas; distinguir processamento offline de audição real.

Usar `./scripts/cargo`, testes públicos do core e checks do workspace. Preservar os 101
testes existentes ou justificar individualmente substituições. Registrar medidas de seek,
frame time e memória, sem inventar orçamento ou desempenho. Testes unitários não comprovam
pixel final, resize nativo ou sincronismo audiovisual.

## Limites de entrega

Não declarar tipos/traits/stubs como suporte multimídia funcional. Se uma dependência externa
impedir a prova, documentar a falha concreta, manter resultado parcial explícito e continuar
as fatias independentes. Sem publicação, push, instalação de shims globais ou alteração de
`tesselapp`. O repositório ainda não tem commit inicial: preservar todos os arquivos existentes.
