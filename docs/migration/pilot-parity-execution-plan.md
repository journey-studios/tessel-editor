# Plano de execução da paridade do piloto no Rust

Data: 2026-11-08. Solicitado pelo usuário: criar o plano das lacunas e direcionar ao
Cursor Claude Sonnet 5.5. Fonte: [auditoria](pilot-parity-audit-2026-11-08.md) e
[catálogo de 59 entradas](pilot-parity-catalog.json).

O status e a atribuição continuam exclusivamente no [backlog](../backlog.md).
As fases abaixo organizam dependências e entregas, sem criar uma fila concorrente.
O primeiro encaminhamento cobre P0 e P1. As demais fases ficam especificadas para
execução posterior, com o contrato e as provas da fase anterior disponíveis.

## Base e limites

- Projeto correto: repositório `journey-studios/tessel-editor`; não é o Tessel RPG nem o monorepo JS.
- Main auditada: `10683c5357c244692063099e13ccb5685385e0fa`. Atualizar referências e
  conferir PRs ao começar; não implementar sobre a branch antiga de documentação.
- Origem: inventário do piloto, snapshot declarado `d40ac3f7c0`. Reabrir blobs fixados
  em modo somente leitura para comparar regras; não alterar, instalar ou executar o JS.
  Se o objeto não estiver disponível, declarar bloqueio de equivalência, sem inventar goldens.
- Uma implementação Cursor Sonnet 5.5, checkout isolado preparado pelo T3, sem subagentes.
  Nenhum checkout novo criado pelo shell. Preservar planos e backlog sujos na raiz.
- Biblioteca Rust é dona das regras; JSONL e GPUI compartilham handlers. Runtime apenas desenha
  o plano. Não criar uma camada Node de edição ou duplicar o domínio na timeline NLE.
- Commit, push e PR da entrega permitidos. Sem merge, deploy, publicação, serviços,
  credenciais novas, limpeza de caches ou cópia de mídia privada.

## Fases e entregáveis

| Fase | Backlog reutilizado | Escopo e saída revisável | Depende de |
|---|---|---|---|
| P0 — equivalência e proveniência | BL036, BL018, BL019, BL082.17 | Fixar casos pequenos da vinheta e comportamento de gesto; manifest com SHA/hash e origem de expectativas. Reconciliar contrato F016 e matriz usando provas identificadas por head, preservando limites históricos | Base atual conferida |
| P1 — completar a vinheta existente | BL082.6, BL082.16 | Preview de candidato durante slider, inserir no frame atual, teclado e orçamento visível. API/JSONL/UI e provas completas da primeira entrega | P0 |
| P2 — inspector neutro e segundo look | BL004, BL014, BL029, BL040, BL082.1 | Modelo de inspector no core, metadados e rótulos pt-br/en/es; escolher `fade` como próximo look simples, implementar só com referência de pixel independente | P1 |
| P3 — parâmetros animados | BL082.2 | Keys/easing/seed, consulta e avaliação por frame, edição atômica e gestos ordenados; depois `filmGrain` com ruído determinístico | P2 |
| P4 — efeitos por plano e transição inicial | BL082.3, BL082.4 | Escopo shot, janelas/handles, dois decks, `crossDissolve` e normalização do dissolve legado; troca de cena sem salto | P2; P3 quando houver keys |
| P5 — duração e cortes de planos | BL022, BL082.5 | trimShot, moveCut, setShotDuration, setEditDuration; ripple/roll e anchor/follow/auto; propagação e recusas explícitas para dados ainda não suportados | P3/P4 e contrato dos domínios propagados |
| P6 — timeline dirigida | BL014, BL028, BL082.6 | Trilhas de FX/transições/shots/keys; handles, mover entre trilhas e delete contextual; todos os gestos semânticos no core | P3–P5 |
| P7 — ampliar famílias dependentes | BL023–BL027, BL017, BL016.1, BL082.1 | Luz/clima/rig/path/follow/narração e transporte dirigido; restante do catálogo por dependência, sem habilitar placeholders | Contrato específico e pré-requisitos de cada efeito |
| P8 — aceite contínuo | BL019, BL043, BL046, BL084 | Equivalência por domínio, pixels/áudio, UI semântica, limites de ponteiro/GPU/áudio ouvido; automatizar gates reais no PR | Executada em cada fase; nunca somente no fim |

O arquivo de catálogo discrimina os 30 looks e 28 transições restantes. Distribuí-los
por P2/P3/P4/P7 conforme tipos, animação e referências exigidas. As 59 entradas não
viram 59 nomes habilitados sem implementação. Cada efeito só fecha com plano, desenho,
referência independente, monitor/export, persistência e acesso pelas três superfícies.

## Primeira entrega para Cursor: P0 + P1

### Contrato funcional obrigatório

1. Gesto de FX pertence a projeto, cena, clipe e revisão de origem. `begin` captura a base;
   cada `update` deriva dela. A consulta/plano de preview consome o candidato, sem escrever
   documento, arquivo ou histórico. Não mover essa regra para o buffer do widget GPUI.
2. Proposta inválida mantém o último candidato legal e fornece erro tipado. `cancel`,
   troca de cena/projeto, undo externo e revisão divergente não podem confirmar valores
   antigos. Definir o comportamento de perda de foco explicitamente, sem dois commits.
3. `commit` com alteração cria um único undo; no-op não cria nenhum. Undo/redo restauram
   valores e pixels. O comando headless usa os mesmos handlers que a UI.
4. Adicionar vinheta usa o frame de edição exibido pelo monitor para a cena selecionada,
   com conversão de tempo no core e duração que caiba na edição. Frame inválido recusa
   atomicamente. Não ler o playhead da sequência como se já fosse o frame do bundle.
5. Teclado usa faixa/passo/quantização compartilhados, e confirmação/cancelamento acessíveis.
   Documentar teclas e testar o adapter GPUI; não anunciar teste semântico como teclado físico.
6. Aviso do orçamento de passes aparece na seção Efeitos e em nó consultável via UI CLI,
   atualizado ao editar, ocultar/remover FX e trocar de cena. Mais de oito passes avisa;
   não recusa, corta passes, reduz qualidade ou oculta o aviso silenciosamente.

### Ownership e documentação

Cursor pode alterar módulos de FX/gestos/plano/comandos no core, adapter GPUI, testes,
prova `scene-fx-ui-proof.py` e contratos F016/F005/F039 quando afetados. Usar arquivos
novos focados se necessário; não iniciar refatoração geral. Runtime só muda se necessário
para consumir o plano de candidato, sem nele implementar regra temporal.

Atualizar `docs/features/effects.md`, matriz de features e relatório de evidências da
entrega. Na branch isolada, atualizar apenas linhas BL082.6/.16/.17 e referências P0
pertinentes; não reformatar o backlog inteiro. Não modificar os documentos sujos na raiz.

### Validação e definição de pronto

- Biblioteca: begin/update/commit/cancel, candidato inválido, no-op, atomicidade, revisão
  obsoleta, troca de cena, desfazer/refazer e frame de inserção. Testar invariantes e fluxo,
  não detalhes internos espelhados pela asserção.
- JSONL sem janela: processo real; candidato consultável e ausência de escrita/undo nos
  updates, commit único e recusa tipada. Socket da janela não substitui headless.
- UI: testes GPUI e instância própria controlada por `scripts/ui`; provar preview antes
  de commit, aviso visível/consultável, teclado do adapter, cancel e troca de cena.
- Pixels: monitor/render/export com vinheta e baseline; ausência/amount=0 preservam imagem.
  Referência independente, não comparar somente dois caminhos que usam o mesmo shader.
- Gates do head final: fmt, `./scripts/cargo check --workspace --locked`, clippy all-targets
  com `-D warnings`, test workspace e build desktop. Usar wrapper; verificar árvore core
  sem GPUI/Wry. Executar runtime/provas de render pertinentes e registrar comandos/saídas.
- Disco: reserva inicial de 16 GiB e piso de 12 GiB; usar lock de GPU e ownership do renderer
  já existente. Se faltar disco, registrar REFUSED_DISK e entregar diff, sem cleanup amplo.
- Abrir uma PR da primeira entrega, linkar ao T3, inspecionar feedback/checks existentes e
  corrigir achados pequenos. Não mergear, não esperar com polling, não iniciar watchers
  como delegado. A última linha identifica PR, head e estado de checks/reviews.

## Fora da primeira entrega

Não iniciar catálogo completo, novos shaders, node scope, transições, ripple/roll,
luz/clima/rig/narração, editor de mapas ou produção de vídeo. Presets vazios,
flashlightReveal, prefetch completo e split/slip de shots ausentes no piloto são
decisões futuras, sem requisito falso de paridade. A autorização de P0/P1 não declara
nenhuma fase entregue antes de implementação e evidência.

## Estado de P2 (entrega do branch `t3/fx-inspector-fade-p2`)

P2 foi implementado e provado em macOS/SwiftShader; o estado por superfície, os comandos e os limites estão em
[F057](../features/fx-inspector.md) e [F016](../features/effects.md#p2-inspector-neutro-e-segundo-look-fade);
o backlog é `docs/backlog.md` (BL004, BL014, BL029, BL040, BL082.1). Resumo do que a fase fechou e do que não:

- Inspector neutro no core (`scene_edit::fx_inspector`), `scene.fx.inspect` (lib, JSONL sem janela, janela),
  rótulos pt-br/en/es com teste sobre os IDs oferecidos, reset por campo e do visual.
- Vinheta migrada para o modelo comum; `fade` (look, saída) no mesmo catálogo/modelo, com corpus de proveniência
  próprio (`scripts/pilot-fade-corpus.py`). `pilot-parity-catalog.json`: `fade` passou de `pendente` para
  `parcial` (sem golden da origem: `ORIGIN_GOLDEN_NOT_COMPARABLE`).
- **Não é paridade integral**: 2 de 59 efeitos; 57 continuam pendentes e sem controle habilitado. Chaves/`amount`
  animado (P3), escopo `shot` e transições (P4) seguem fora. Windows, ponteiro físico (BL043), GPU real e áudio
  ouvido não foram provados.
