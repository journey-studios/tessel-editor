# Integração da rodada H/I/J/K + correção L

Estado: código integrado e gates locais passando. Fonte de status por item:
[backlog compartilhado](../backlog.md). A base do PR anterior é `6cc4392`.
Esta rodada não completa todos os building blocks nem migra as famílias visuais.

## Resultado por frente e superfície

| Frente | Entrega | Lib | Headless | Janela/socket | Limite |
|---|---|---|---|---|---|
| H / F037 | ScenePlan puro por frame, câmera/nós/matrizes/ordem/assetIds/capabilities | V no subconjunto | P | P | Sem renderer; mapas/luz/clima/rig/splat dirigido explicitamente não avaliados |
| I / F038 | Manifesto/closure, preflight e materialização de PNG/SPZ/mix | Parcial, fatia testada | P | P | Host/pacote/intercâmbio não ligados; guarda de pacote incompleto permanece |
| J / F039 | Árvore/seleção, avaliação de frame, gesto com candidato e commit único | V no subconjunto | V | P | Sem controles/handles/ponteiro; IDs entre sessões e observabilidade registrados |
| K / F032 | +47 casos do oráculo (base 55 preservada), JOIN/FPS/seek/matrizes | Parcial | N/A (ferramenta Node) | N/A | Câmera por Three isolado, não método privado do runtime; divergências declaradas |
| L / F005/F024 | Preview de slider sem micro-undos; commit/release, cancel/no-op | V no ciclo | V | Socket V; 13 testes de input GPUI | Mouse físico ainda não provado; ações de a11y e thumb no cancel têm pendências |

O mesmo recurso é identificado por assetId no plano e no manifesto. O teste de integração
`scene_plan_resources` verificou 4 bundles × 4 consultas, com plano parcial explícito quando
necessário. Isso não prova render nem transporte completo de domínios preservados.

## Gates locais executados

Todos os comandos Cargo de build/test/lint usaram `scripts/cargo` com a fila local e
`MBX_GC_MAX_TOTAL_SIZE=64GiB`; sem clean/gc manual nem HOME alterado em wrappers.

- `fmt --all -- --check`: passou.
- `clippy --workspace --all-targets --locked -- -D warnings`: passou.
- `TESSEL_REQUIRE_RUNTIME=1 test --workspace --locked`: **718 passaram, 0 falharam, 2 ignorados**, 45 grupos.
  Ignorados: ZIP64 de 4,5 GiB; não acionados nesta rodada.
- `check --workspace --locked` e `build -p tessel-editor --release --locked`: passaram.
- `cargo tree -p tessel-editor-core`: sem GPUI/wry (core puro).
- Node: `oracle.spec.mjs` + `oracle-extended.spec.mjs`: **45/45**; `oracle.mjs verify`
  e `extended.mjs verify`: **55 + 47 casos regenerados contra a origem**.
- Executável headless: sessão de 13 requests de cena (attach/tree/select/evaluate/gesto/get/undo/redo)
  e sessão de 12 requests de slider; preview sem entradas, commit uma, undo/redo conferidos.
- `scripts/slider-live-proof.py`: **12/12** na janela nativa com HOME/socket isolados;
  [resultado e hashes](../slider-gesture-live-proof.json). Sem ponteiro físico nem screenshot.
- O app do usuário recebeu o release com a correção do slider após snapshot local e sem jobs
  ativos; nome, tempo e duração foram conferidos. O snapshot do usuário fica fora do Git.

O pipeline do PR tem CodeRabbit; os testes acima são locais, não CI. Não chamar um check
neutro/ignorado de revisão executada. Artefatos numéricos têm verificação própria de proveniência.

## Ajustes na integração

- Exports de scene_plan/scene_interaction/scene_resources e directed_resources; inicialização
  do estado efêmero de cenas no loader do projeto.
- Duas expectativas J corrigidas: gesto verifica o alvo antes da existência do nó.
- Helpers GPUI separados para evitar empréstimos mutáveis aninhados; janela ativada antes de
  simular perda de foco; press do thumb só abre gesto quando houver valor.
- Slider limpa drag GPUI anterior num novo press. Core mantém uma entrada por gesto, com
  conflitos de revisão/alvo tratados explicitamente.
- Publicação de recursos falha sem fallback check-then-rename; regressão falhou antes e
  passou depois. Contenção requer controle exclusivo das raízes; não há promessa de defesa
  contra troca concorrente hostil de diretórios-pais.
- IDs sugeridos pelas frentes foram reconciliados no backlog; BL062 é exclusivamente slider.

## O que ainda destrava o primeiro fluxo completo

1. BL007/BL008: consumidor real de ScenePlan no runtime e export dirigido.
2. BL011: ligar recursos ao host/package/restore/interchange e provar abertura sem origem.
3. BL014/BL015: inspector/overlays de cena, foco e gestos visuais sobre os mesmos comandos.
4. BL016/BL017: transporte e áudio/narração do modelo dirigido.

O catálogo continua com oito propriedades. A UI de mídia/timeline existente continua funcional,
mas não é evidência da UI de DirectedScene. Não foram portados luz/clima/rig/efeitos nesta rodada.
