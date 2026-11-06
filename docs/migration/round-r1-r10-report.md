# Integração R1–R10

Base: `914c830`, branch `feat/directed-scene-next-delivery`. Esta entrega reúne as dez frentes e a correção posterior de cleanup. Status e pendências permanecem em [backlog](../backlog.md); contratos por superfície na [matriz](../features/README.md).

## Comportamento entregue

- Export dirigido real em presets do core, incluindo Full HD e UHD, com tempo racional, snapshot imutável, originais verificados, jobs e política explícita de omissão de áudio. `PlanTimeline` é reutilizada por job. F042 tem lib, JSONL, socket e diálogo `flow.export_scene`; o cabeçalho escolhe esse diálogo na aba Cena.
- Anexar/carregar/reparar cena pelo diálogo, botão do inspector ligado ao mesmo fluxo e cabeçalho compacto abaixo de 1040 px. Layout de árvore e propriedades adaptável a docks estreitos.
- IDs opacos de gesto por sessão, seleção podada no undo/redo, summary de interação e validação do orçamento sem mutação parcial.
- Substituição de pacote recuperável em falha de publicação; import/adopt pendente com cleanup de arquivos próprios, claims compartilhadas no processo, proteção por conteúdo e rollback de publicação em panic.
- Lifecycle dirigido no WKWebView preserva slots ao perder contexto, libera slots em falhas com página viva e descarta mensagens da página anterior no reload. Corrida do teste Spark removida com gate determinístico, sem alterar runtime.

## Revisão de integração

O coordenador ligou o renderer no host desktop, o diálogo de export e o botão Anexar do inspector. A guarda pública de `SceneExportSnapshot::plan_with` agora recusa uma timeline pertencente a outro snapshot (`stale_scene`); o teste novo falhou antes da correção.

O golden de planos falhou no primeiro gate completo por `serde_json/preserve_order` unificado a partir de GPUI, embora passasse isolado. O comparador canonicaliza a ordem de chaves, preserva números/valores e usa os mesmos digests anteriores, sem regeneração.

## Validação final

Gate local final passou em macOS. Evidências, hashes e proveniência: [resumo](round-r1-r10-results/summary.json).

| Verificação | Resultado |
|---|---|
| Workspace fmt/check/clippy `-D warnings` | passou |
| Rust, workspace com `TESSEL_REQUIRE_RUNTIME=1` | 1027 passaram, 8 ignorados, 64 grupos |
| Runtime, concorrência padrão | 85/85 |
| Build workspace debug e editor release | passaram |
| Export independente CLI/socket/diálogo | 23/23 fases, 570 checks, [registro](round-r1-r10-results/export.json) |
| WKWebView lifecycle dirigido, Apple GPU | 218/218, [registro](round-r1-r10-results/lifecycle.json) |
| Diálogo de importação/reparo | 33/33, [registro](round-r1-r10-results/import.json) |
| Layout/capturas nativas | 50/50 capturas, [registro](round-r1-r10-results/visual.json) |

As oito entradas Rust ignoradas são duas medições do planner, uma medição de cleanup, dois testes ZIP64 grandes e três entradas do harness de export executado separadamente acima. Não foram executados os casos de ZIP64 >4 GiB ou as medições ignoradas nesta rodada final.

Os binários de render/editor têm `older_than_sources:false` na prova de export. O coordenador abriu as capturas `selected-min-dark`, `selected-min-light` e `selected-panel100-light-inspector`: cabeçalho sem sobreposição nos dois temas e botão Anexar dentro do dock estreito. Isso não significa inspeção individual das 50 imagens nem prova de ponteiro físico. Header `node:16` e preflight do diálogo não foram dirigidos pela prova R10.

A evidência precede o commit, por isso os registros incluem base e árvore suja; `summary.json` registra digest das fontes e hashes dos binários finais. Somente documentação e evidências foram alteradas após os testes. CI e revisão da PR são etapas distintas deste gate local.

## Limites materiais

- Não há áudio dirigido; conteúdo que declara áudio é recusado salvo `audio:false`, com aviso tipado. Não há prova de som ouvido.
- Claims de cleanup só coordenam resultados no mesmo processo. Dois editores no mesmo workspace precisam de BL111. Hash de arquivos grandes no descarte ainda pode bloquear UI (debug: 256 MiB em 8,7 s); BL094/BL112 tratam o descarte assíncrono.
- Pacote de pasta ainda tem intervalo entre dois renames e recuperação manual após queda; BL098. Mapas/grades/narração não são transportados integralmente (BL077).
- Não há garantia de câmera/splat em movimento a 1–4 ms, seek-to-photon, driver reset real, Linux/Windows ou benchmark final do ganho de PlanTimeline no export.
- Resposta tardia de asset de slot reutilizado e prova nativa acima de 32 recursos continuam em BL102/BL103.
- Export longo, bytes de recursos alterados após aprovação e comparação com o runtime da origem têm pendências próprias. O corpus é sintético e a referência de pixels é independente.

## Publicação e revisão

Implementação e evidências publicadas na [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), commit `f6311d20bcdc6768b39756f5531e5f44850f983a`. CodeRabbit pulou a revisão por selecionar 121 arquivos, acima do teto de 100; o comentário também cita capacidade/créditos indisponíveis. [Status do serviço](https://github.com/journey-studios/tessel-editor/pull/4#issuecomment-6669704948). Não houve aprovação do CodeRabbit nem merge. A configuração dessa execução veio da base `914c830`, sem os filtros novos de artefatos adicionados nesta PR. A entrega permanece em uma PR, conforme a regra do projeto; não foi dividida ou ocultada para contornar a revisão.

## Merge confirmado

O usuário autorizou o merge apesar da revisão pulada. A PR #4 foi mergeada na main em `c099f102d230271923684bc4e27a970a2bdfbe2a`; o checkout foi atualizado para esse merge. O bloqueio de revisão descrito acima é histórico e não significa aprovação do CodeRabbit. Status atuais estão no backlog reconciliado.
