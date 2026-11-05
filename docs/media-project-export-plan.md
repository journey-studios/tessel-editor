# Plano — Spark, mídia, projetos portáteis e exportação de vídeo

Status: **planejamento, implementação não iniciada**. Data: 2026-11-04.
Escopo aprovado: Spark, import de arquivos/splats, salvar e transportar projetos,
intercâmbio entre projetos, presets de vídeo, undo/recuperação, jobs, proxies,
preflight, reprodução e consistência entre preview e export.

Este documento define a implementação futura. Os nomes de arquivo, APIs e comandos
abaixo são propostas, não endpoints entregues. Cada fatia deve atualizar seu contrato
em `docs/features/` e a matriz, seguindo a skill `tessel-feature-contract`.

## 1. Como outros editores transportam projetos

| Editor | Mecanismo documentado | O que adotamos como referência |
|---|---|---|
| Premiere | Project Manager coleta arquivos numa nova pasta ou consolida/transcodifica; pode excluir clipes não usados e incluir handles e previews | Separar coleta de originais de otimizações; escopo por sequência e estimativa de espaço |
| Final Cut Pro | Consolida mídias em uma localização de armazenamento, selecionando originais, mídia otimizada e proxies | Projeto pode trabalhar com referências externas e consolidar dependências para transporte |
| DaVinci Resolve | Export Project Archive, distinto da exportação do documento do projeto | Distinguir projeto leve de archive que acompanha as mídias |

Fontes primárias: [Adobe Project Manager](https://helpx.adobe.com/premiere/desktop/organize-media/transfer-files/copy-transcode-or-archive-project.html),
[Apple — Consolidate media](https://support.apple.com/en-gb/guide/final-cut-pro/ver9c7660349/mac),
[Blackmagic — guia oficial Resolve 20](https://documents.blackmagicdesign.com/UserManuals/DaVinci-Resolve-20-Beginners-Guide.pdf?_v=1757574012000).
O PDF da Blackmagic foi localizado no índice oficial, mas não pôde ser lido integralmente
nesta pesquisa pelo limite de download; não pressupor detalhes adicionais de suas opções.

Conclusão de projeto: **consolidar dependências é a operação essencial**. Zip é apenas um
invólucro de transporte. Salvar um documento não equivale a backup das mídias, e exportar
um vídeo não preserva um projeto editável.

## 2. Baseline verificada e decisões

- Core público sem GPUI; modelos de projeto/assets/tracks/clipes, plano de render e bridge v1.
- Runtime Three.js 0.187.0 compartilhado por WebView e worker Chrome sem janela do editor.
- Splat atual: renderer próprio de prova, formato `.splat`; será substituído por Spark.
- Import atual é cadastro/probe parcial; não há persistência, pacote ou encoder de vídeo.
- Áudio atual é mix offline; playback/saída em dispositivo continuam pendentes.
- Baseline anterior: 187 testes Rust e 13 JS. Não remover cobertura sem justificar substituição.

**Spark é a escolha aprovada.** Referência local confirmada, somente leitura:
`~/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/package.json`
usa `@sparkjsdev/spark` **2.4.0** e Three.js **0.187.0**, versão igual à deste runtime.
`src/runtime/splat-stage.ts` instancia `SparkRenderer`, espera `SplatMesh.initialized`
e estabiliza o sort antes de renderizar cada câmera. Os comentários registram limitações
de composição entre splats e planos transparentes. Esses comentários são evidência do
outro projeto, não validação no editor novo.

Usar 2.4.0 como candidato inicial fixado e validar compatibilidade/artefatos locais;
não atualizar para uma versão diferente só por ser mais recente. Estudar e adaptar o
contrato do estágio existente, sem portar o motor inteiro ou copiar assets privados.

## 3. Contratos e divisão de responsabilidades

- **Core:** documento canônico, IDs, tempo, regras de edição, referências de assets,
  dependências, políticas de import/merge, presets validados e planos de jobs/render.
- **Adapters de serviços:** filesystem, hash, probe/decode, cache, Spark/JS, áudio e FFmpeg.
  Executam planos do core, reportam progresso e erros; não repetem regras de edição.
- **UI e CLIs:** traduzem entradas para a mesma API, apresentam resultados e permitem
  consultar/cancelar jobs. O seletor nativo de arquivos é apenas uma entrada de caminhos.
- Nenhuma operação de domínio depende de seleção de UI, janela aberta ou socket desktop.
- Paths absolutos, handles de GPU, entidades GPUI, estado de execução de jobs e cache
  não são a identidade persistente do projeto. Persistir apenas os dados necessários.
- Operações de I/O longas nunca bloqueiam a thread da UI. A versão/revisão editada é
  capturada no início do export; edições posteriores não alteram silenciosamente o job.

## 4. Documento, workspace e pacote

### 4.1 Salvar e abrir

Proposta: `nome.tessel.json` como documento versionado. Deve conter identificador de
projeto, `schema_version`, revisão, metadados, registro de assets e dependências,
sequências, FPS racional, canvas, tracks, clipes, trims, transformações, texto,
câmeras/animações e definições de efeitos quando existirem, além de preset selecionado.

Dados de apresentação (Dock, seleção, zoom do editor) ficam separados ou opcionais;
ausência deles não impede render. Resolução/FPS de export são overrides explícitos.

Assets têm identidade lógica e hash de conteúdo separados. Arquivos idênticos podem
compartilhar bytes preservando nomes e identidade de edição. Origem externa é informação
de relink, não requisito para abrir um pacote completo. Caminhos relativos resolvem
a partir do documento; evitar dependência do cwd do processo.

Save escreve arquivo temporário no destino e só substitui a versão válida após sucesso.
Load valida e constrói estado candidato antes de substituir o projeto aberto. Versões
futuras incompatíveis falham com mensagem; migrações conhecidas são testadas e preservam
o original. Snapshot persistente não serializa o interner de IDs nem structs internas.

### 4.2 Workspace de trabalho e referências

Import pode **referenciar original** ou **copiar para mídia gerenciada**. Proposta inicial:
copiar para o workspace por padrão, com opção explícita de referência para grandes acervos.
Em ambos os casos, nunca mover/deletar originais do usuário como efeito de import/package.
O diretório de cache é descartável e separado das mídias necessárias.

### 4.3 Empacotar e restaurar

Proposta: diretório consolidado, com export opcional em arquivo `.tesselpack` baseado
em ZIP64, adequado a mídias maiores que 4 GiB. Estrutura lógica:

```text
project.tessel.json
manifest.json
media/<hash>/<nome-original>
resources/                 # texturas, LUTs e fontes permitidas pelo projeto
proxies/                   # opcional
```

- Padrão: todas as dependências de mídia do documento; opção por sequências selecionadas
  deve incluir o fechamento transitivo de cenas, splats, texturas, áudio, fontes e efeitos.
- Copiar originais completos na v1. Recorte/transcode com handles é evolução posterior;
  não recortar splat ou modificar o original para diminuir o pacote.
- Manifest registra paths, tamanhos, hashes e requisitos de schema/runtime/Spark/efeitos.
  Informações não serializáveis ou dependências ausentes aparecem no preflight.
- Caminhos do pacote são internos e relativos. Caches/previews ficam fora por padrão;
  proxies são opcionais. Nunca substituir original por proxy sem declarar o pacote incompleto
  para render final. Recursos redistribuíveis, como fontes, precisam de política explícita.
- Calcular tamanho/espaço estimado; copiar e verificar em staging, publicar só ao concluir.
  Falha ou cancelamento preserva origem e destino anterior. Não sobrescrever silenciosamente.
- Restaurar em destino escolhido com verificação de manifesto/hash; validar paths,
  links e limites de extração. Rejeitar escapes do diretório e colisões ambíguas.
- Projeto com referências quebradas pode abrir offline para edição/relink; pacote marcado
  completo e export final exigem dependências resolvidas. Pacote parcial exige opção explícita.

### 4.4 Importar conteúdo de outro projeto

Operação distinta de abrir/restaurar: selecionar assets, sequência ou tracks, mostrar
plano de import e remapear TODOS os IDs/referências transitivas antes de aplicar uma
transação. Hash pode evitar cópia de bytes, sem fundir automaticamente objetos editáveis.

Para a v1, importar uma sequência preserva seu FPS/canvas em uma sequência própria do
destino. A interface precisa escolher a sequência ativa. Mesclar tracks/clipes numa
sequência existente exige política explícita de offsets, FPS e enquadramento; nunca
arredondar silenciosamente tempos ou sobrescrever entidades com mesmo ID/nome.
Adicionar suporte de múltiplas sequências ao modelo é pré-requisito desse comportamento.

## 5. Importação e Spark

- Import em lote por diálogo/drag de arquivos, CLI e API; plano antes da aplicação,
  resultados por arquivo, cancelamento e política clara de sucesso parcial.
- Probe de conteúdo via adapter (FFprobe para áudio/vídeo): streams, duração, timestamps,
  FPS racional/VFR, orientação, cor, dimensões, canais, taxa de amostragem e tamanho.
- Identificar pelo conteúdo, não só extensão. Definir limites e metadados indisponíveis
  explicitamente; não confiar em parâmetros fornecidos para um arquivo incompatível.
- Originais imutáveis; hash em streaming, detecção de duplicados, thumbnails/waveforms
  e proxies assíncronos. Invalidar derivados quando fonte/configuração/backend mudarem.
- Relink por arquivo/pasta, com confirmação de correspondência, duração e streams;
  nomes iguais não bastam. Preservar clipes e overrides do usuário.
- Primeiro lote a garantir: PNG/JPEG, WAV, MP4 H.264/AAC e MOV conforme decoders disponíveis;
  outros formatos entram por capacidades verificadas, não promessa universal.
- Spark: garantir primeiro `.splat`, PLY Gaussian e `.spz` com fixtures reais e versões
  explícitas. PLY genérico de pontos não deve ser aceito como Gaussian splat silenciosamente.
  Outros formatos documentados pelo Spark ficam sujeitos a capability/prova individual.
- Assets Spark, workers e WASM devem funcionar localmente/offline, sem CDN. Decoder/load
  deve liberar recursos ao trocar projeto e produzir erro estruturado na falha.
- Testar transparência, ordem de composição, múltiplos splats, câmera, coordenadas/escala,
  seek fora de ordem, load cancelado e perda de contexto. O frame só está pronto após
  estabilização do renderer/sort para a câmera e recursos daquele pedido.

Documentação de referência: [Spark loading](https://sparkjs.dev/docs/loading-splats/)
e [PackedSplats](https://sparkjs.dev/docs/packed-splats/). Suporte declarado pela biblioteca
não comprova suporte em WKWebView/Chrome e no pipeline de render deste editor.

## 6. Exportação, presets e reprodução

Pipeline: snapshot do projeto → preflight → plano por frame → render em resolução final
→ mix offline no mesmo intervalo → encoder/muxer FFmpeg → verificar saída → publicar.
Render em 4K não é ampliar screenshot/preview de baixa resolução. Monitor fechado não
interrompe o job. Backpressure e recursos limitados evitam reter todos os frames em RAM.

Presets de saída iniciais propostos:

| Preset | Dimensões | Saída inicial |
|---|---|---|
| Full HD horizontal | 1920×1080 | MP4 H.264 + AAC |
| UHD 4K horizontal | 3840×2160 | MP4 H.264 + AAC |
| Full HD vertical | 1080×1920 | MP4 H.264 + AAC |
| Quadrado | 1080×1080 | MP4 H.264 + AAC |
| Personalizado | Validado pelo backend | Codec/container suportados |

FPS preserva a sequência por padrão, com override explícito. Preset versionado também
define codec, qualidade/bitrate, áudio, pixel format, cor, política de fit/crop/pad,
intervalo e tratamento de alpha. Salvar presets do usuário e clonar built-ins; não
confundir preset do produto com o parâmetro de velocidade do encoder.

Primeira entrega: SDR Rec.709, política documentada de entrada sRGB/linear/saída,
MP4 sem alpha e WAV mixado em 48 kHz antes de AAC. Converter de fato os pixels quando
necessário; tags de cor sozinhas não são conversão. HDR, HEVC e masters com alpha/ProRes
podem seguir como extensões após prova de compatibilidade e distribuição do encoder.

FFmpeg/FFprobe devem ser descobertos/versionados como adapters, com diagnóstico de codecs
disponíveis. Encoder hardware é otimização opcional, com fallback explícito; não comprometer
paridade de resultado ou assumir que a máquina oferece o codec. Definir empacotamento e
licenças dos binários antes de distribuir o aplicativo, sem bloquear POC local.

Tempo offline é determinístico: congelar recursos/fontes/random seeds, aguardar decode e
Spark, mapear frame e amostras ao mesmo intervalo. Export não captura playback em tempo real.
Verificar duração, frame count/timestamps, resolução, streams e A/V sync com FFprobe mais
inspeção de frames e audição; exit 0 não basta.

Playback interativo usa o mesmo mapa de tempo e composição, relógio sincronizado com
saída de áudio, seek assíncrono e política de frames atrasados. Proxies são só para preview;
export usa original por padrão. Resolver overlays/foco/resize da WebView como gate da UI.

## 7. Jobs, recuperação e preflight

Tipos: import, hash/probe, proxy, package, project-import e video-export. Estado consultável:
queued/running/succeeded/failed/cancelled, etapa, unidades concluídas/total, progresso,
warnings e erro tipado. ETA é estimativa; cancelamento tem acknowledgement e cleanup.

Limitar concorrência/CPU/GPU/memória/disco. Renderiza um snapshot imutável; fechar a janela
não deve matar um job headless independente. Definir no desktop o comportamento de sair
com jobs ativos. Não prometer retomar um encoder interrompido: v1 reinicia o export;
artefatos intermediários só são reaproveitados se comprovadamente íntegros e compatíveis.

Autosave mantém revisões separadas; save explícito não é substituído por recovery silencioso.
Após crash, oferecer recuperar/descartar sem sobrescrever o original. Undo/redo atua sobre
transações de domínio; import em lote/project-import têm uma unidade de undo clara.
Undo remove referências gerenciadas, sem apagar originais ou bytes ainda utilizados.

Preflight valida: dependências ausentes/alteradas, hashes, schemas, versões de efeitos,
fontes, codecs, limites de GPU/textura, canvas/FPS/intervalo, espaço de staging e destino,
permissão de escrita, proxies usados como originais e dependências não transportáveis.
Retorna relatório estruturado igual na UI/CLI/lib, com blockers e warnings distinguíveis.

## 8. Fatias de implementação e dependências

Todas começam pendentes. IDs Pxx são etapas do plano, não features marcadas entregues.

| Etapa | Entrega | Depende de | Aceite mínimo |
|---|---|---|---|
| P01 | Schema de projeto, IDs e múltiplas sequências; serialização canônica | Base existente | Round-trip mantém plano e referências; documento inválido não altera sessão |
| P02 | Save/load, autosave/recovery, dirty state e undo/redo | P01 | Reabrir reproduz projeto; crash simulado recupera revisão; undo/redo por lib/CLI/UI |
| P03 | Infra de jobs, staging/cancelamento, filesystem/cache e capabilities | P01 | Job consultável/cancelável sem UI; erros preservam arquivos anteriores |
| P04 | Spark 2.4.0 e contratos de render no runtime compartilhado | Base + P03 | `.splat`/PLY/SPZ, câmera e seeks fora de ordem geram frames prontos e verificáveis |
| P05 | Import/probe/copy/reference/dedup/relink em lote | P01–P03 | Import real por lib/CLI/UI, arquivo movido relinkado, falha por item documentada |
| P06 | Thumbnails/waveforms/proxies e decoder de vídeo determinístico | P03/P05 | Cache invalida corretamente; proxy não muda tempo nem substitui original no export |
| P07 | Package/consolidate/restore com manifest e integridade | P02/P03/P05 | Abrir em outra pasta sem origens disponíveis; todos os assets necessários resolvidos |
| P08 | Intercâmbio entre projetos e merge transacional | P01/P07 | Importar sequência com IDs colidentes remapeia toda dependência sem mutar origem |
| P09 | Playback/áudio em dispositivo e gates nativos de WebView | P04/P06 | Audição e A/V sync medidos; overlay/foco/resize provados por ações semânticas |
| P10 | Export curto 1080p com áudio e FFmpeg | P02–P06 | Mesmo snapshot pela lib/CLI/UI produz vídeo íntegro; preview/frames finais comparados |
| P11 | Presets personalizáveis, vertical/quadrado e UHD 4K | P10 | Resolução/FPS/fit/cor corretos, 4K real, memória limitada, cancelamento e disco cheio |
| P12 | Prova completa de transporte/intercâmbio/export e regressão | P07–P11 | Projeto em máquina/pasta limpa com Spark, áudio, texto e vídeo exporta resultado esperado |

P04 pode evoluir em paralelo com persistência após fixar contratos. P09 é gate de aceitação
do preview, mas não precisa bloquear o desenvolvimento do export offline P10.
Não estimar prazo antes de executar P01/P03 e medir Spark/4K. Este plano não inicia delegações.

## 9. API, CLI e UI por feature

Vocabulário proposto: `project.save|open|recover`, `history.undo|redo`, `media.import|relink`,
`project.package|restore|import`, `job.get|list|cancel`, `export.preflight|start`,
`preset.list|save`. Gerar IDs de job e respostas versionadas; exemplos reais entram nos
contratos da implementação. Clientes Rust não devem precisar montar argumentos de shell.

Na UI: novo/abrir/salvar/recuperar, import em lote, biblioteca com status de assets, relink,
package/import-project, fila de jobs e export com presets/resumo/preflight/progresso/cancelar.
Adicionar comandos semânticos de abertura/foco de menus, popups e diálogos necessários
para teste, sem depender de cliques por coordenadas.

Desmembrar F020 em contratos de persistência, histórico, pacote, intercâmbio, jobs,
presets e export à medida que cada fatia começar; preservar F020 como índice histórico.
Atualizar F019 (mídia), F018 (playback), F021 (Spark/viewport), F015 (projeto/seq) e criar
linhas próprias para capacidades novas. Nunca marcar I/V apenas por concluir um plano.

## 10. Provas de aceite e definição de pronto

Fluxo principal: criar projeto → importar splat, vídeo, imagem e áudio → adicionar texto,
tracks e câmera → editar/undo → salvar/reabrir → empacotar → tornar originais indisponíveis
→ restaurar em outro diretório → importar a sequência em projeto com IDs colidentes →
preview com áudio → exportar o mesmo trecho em 1080p, vertical e UHD 4K.

Cada etapa precisa de fluxo público pela lib, processo CLI sem janela e integração UI;
teste de core não prova interação nativa. Resultado deve funcionar após reiniciar processos,
sem fixtures ocultas, paths absolutos de desenvolvimento ou sessão do projeto de origem.

Casos negativos: mídia ausente/corrompida/modificada, duplicate ID/nome, hash errado,
package truncado, incompatibilidade de schema, fps misto/VFR, duração fracionária de áudio,
frames fora de ordem, decoder/Spark/encoder indisponível, cancelamento, crash e disco cheio.
Provar streaming com mídia grande e arquivos acima de 4 GiB; não limitar o teste a fixtures
pequenas. Testes longos devem ser explícitos e ter orçamento/limpeza de artefatos próprios.

Medir seek, frame time, sort Spark, render 4K, memória de CPU/GPU, espaço temporário,
tempo de encode e diferença preview/export. Fixar hardware/runtime/GPU/encoder usados na
evidência; definir tolerâncias, sem prometer hashes iguais entre GPUs.

Manter checks Rust pelo wrapper Cargo e runtime via pnpm, testes existentes e novos,
verificação FFprobe, frames reais e revisão audiovisual. Relatório final distingue
processamento offline, playback ouvido, prova nativa, plataformas testadas e lacunas.

## 11. Fora desta rodada

Não inclui importar formatos de projeto de Premiere/Resolve/Final Cut, colaboração cloud,
render farm ou migração do motor inteiro do editor antigo. Intercâmbio aqui é entre projetos
Tessel. Conversão de mídia com trim/handles, HDR, novos codecs e retomada de encoding podem
vir depois, sem impedir o pacote completo de originais e o export inicial SDR.
