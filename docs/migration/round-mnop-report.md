# Integração M/N/O/P — cena dirigida de ponta a ponta

Base main 192e766, branch feat/directed-scene-end-to-end. Quatro implementações Cursor Sonnet 5.5 recebidas e integradas pelo coordenador. Este relatório distingue a entrega dos agentes dos checks na árvore combinada. Status operacional único: [backlog](../backlog.md).

## Resultado

- M / F041: consumidor ScenePlan em Three.js, SceneSlots e SceneRenderer. Composição de imagens, câmera, matrizes e animação já avaliada; demais domínios continuam recusados.
- N / F040: recursos verificados por hash, base gerenciada, geração/orçamento, jobs, adopt/relink, pacote/restauração e import de cenas. Lease cooperativo; escritor hostil permanece fora da garantia.
- O / F039-UI: árvore e oito propriedades por entidade, seleção, reset e candidato de gesto; slider confirma um undo, Escape cancela.
- P / BL019: corpus sintético, modelo independente, harness e provas por superfície.

## Integração adicional do coordenador

1. `SceneCommand::Plan`, parser scene.plan e capabilities: consulta com gates e candidato de gesto vivo, sem histórico. A operação do corpus deixou de ser ignorada.
2. `headless::output_geometry` extraído da validação já existente para reuso de limites de dimensão/probes.
3. `scene_operation`: prepara o plano pelo comando do core e converte o ResourceResolver de N em mapa de M, sem hash/I/O pesado na UI.
4. `SceneHost`: cena e operações de mídia na mesma sessão JSONL tessel-render; scene.render usa arquivo verificado pelo MediaHost.
5. `scene_monitor_host`: mesmo SceneSlots/frame_render_message no WebView assíncrono; confirmação de asset, coalescência de seek, ids separados, PNG e resposta explícita após frame real, cancelamento no reset.
6. Monitor integrado por botão e monitor.show_scene. Voltar a Sequência/Fonte limpa a apresentação dirigida. Perda de contexto/reload esquece slots e pede quadro atual quando o runtime volta. A prova nativa forçada desse lifecycle ainda é BL092.
7. Harness atualizado para nomes reais, job concluído/aplicado e expansão semântica da árvore. Nenhum caminho de arquivo é tratado como id de UI. Pacote restaurado é renderizado sem origem.

## Checks na árvore combinada

- Workspace Rust com TESSEL_REQUIRE_RUNTIME=1 e testes serializados: **827 passaram, 0 falharam, 2 ignorados** (ZIP64 grande), 52 grupos. Depois foi acrescentado um teste de candidato de scene.plan: a suíte scene_flow foi repetida e passou **8/8**. Não somar esse teste como se o workspace completo tivesse sido repetido.
- Desktop dentro do gate: **140 testes** (inclui três novos do adapter assíncrono: load/ack, coalescência/reset e recusa sem mutação). Inputs GPUI não provam ponteiro físico.
- clippy workspace/all-targets com -D warnings: limpo. fmt --check e build release editor/render: passaram.
- Core dependency tree sem GPUI/wry; Cargo.toml/lock não precisaram mudar.
- Provas finais do release e runtime: ver artefato compacto e seção de resultado final abaixo, preenchidos após a execução.

## Limites preservados

- Export dirigido BL008 ainda não está implementado; export comum existente passou no gate, inclusive 4K. A cena dirigida não vira automaticamente clipe da timeline.
- Anexar/carregar cena ainda exige comando; diálogo visual dedicado BL086 pendente.
- Sem inspeção visual do layout (BL087), ponteiro físico, seek-to-photon ou áudio ouvido.
- Context loss/reload dirigido no WebView exige prova BL092; Node e host roteirizado são evidências distintas.
- Renderer dirigido suporta imagens estáticas; não significa que luz/clima/rig/splat dirigido/efeitos tenham sido migrados. Spark do viewport comum continua separado.
- Fundo de saída preto opaco e sem mipmaps; BL090. Nenhuma comparação de pixels com runtime original; modelo independente validado contra corpus da origem.
- Recursos de mapas/f32/narração seguem parciais; lease é cooperativo; Unicode/Windows, import obsoleto e replace de pacote têm pendências no backlog.
- Cache de asset explícito pode ser fornecido por biblioteca sob responsabilidade do chamador; caminho padrão integrado usa verificação de bytes do MediaHost.

Não reiniciamos nem alteramos o projeto/janela do usuário. Provas nativas usam instâncias próprias e HOME/socket isolados, sem coordenadas.

## Resultado final da prova combinada

**408/408 checks**, exit 0, no release de editor/render e executável headless atualizado (nenhum sinalizado como antigo às fontes). Corpus/modelo 14, headless 80, plano 60, pacote/restore/pixels sem origem 7, render headless 40, seek fora de ordem 48, troca de recurso 36, fluxo por socket 80, pixels/monitor nativos 41 e árvore semântica 2.
Artefato compacto com hashes dos binários/harness/corpus e tolerâncias: [scene-integration-results.json](scene-integration-results.json). Os pixels são do canvas/readback, não prova de fóton na tela nem inspeção visual.

Comando reproduzível (após build):

```sh
TESSEL_VIEWPORT_INSPECT=1 python3 scripts/directed-scene-proof/proof.py \
  --phases corpus,model_vs_origin,headless_flow,scene_plan,package_restore_no_origin,render_headless_pixels,render_seek_out_of_order,resource_swap,socket_flow,socket_render_pixels,ui_semantic \
  --bin-render target/release/tessel-render --bin-headless target/debug/tessel-headless \
  --bin-editor target/release/tessel-editor --report /tmp/scene-proof.json
```

`context_loss` e `export_directed` foram explicitamente não executados. O conjunto selecionado verde não fecha essas pendências.

Runtime final reexecutado pelo coordenador: **79/79**, TESSEL_REQUIRE_RUNTIME=1, execução serializada sob lock da bancada, sem skips. A falha intermitente com concorrência padrão continua BL091; não foi mascarada como resolvida.
