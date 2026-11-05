# Próxima rodada da base — H/I/J/K

Base: `6cc4392` + backlog comum. Rodada autorizada em 2026-11-05.
Quatro agentes Cursor `claude-sonnet-5-5`, checkout compartilhado com ownership exclusivo.
A tentativa de `isolation: worktree` com target.projectId do tessel-editor foi recusada pelo T3:
“A local task runs in this thread's project.” Esta conversa pertence ao projeto tesselapp.
Não criar worktrees no shell, não mudar o projeto pai nem editar tesselapp como contorno.
Cada agente lê AGENTS, a skill de contrato e `docs/backlog.md`. Sem source edit na origem.

## Ownership e integração

| Frente | Backlog | Entrega desta fatia | Ownership exclusivo | Feature |
|---|---|---|---|---|
| H | BL006 (BB05) | Plano puro, tipado e serializável do frame de cena dirigida: câmera/nós/matrizes/ordem/recursos requeridos e versão/capabilities; recusar domínios não avaliados explicitamente | `crates/core/src/scene_plan/`, `crates/core/tests/scene_plan*`, `docs/features/scene-plan.md`, relatório H | F037 |
| I | BL009/BL010/BL011 (BB06 inicial) | Manifesto de recursos e dependências com IDs estáveis; resolução/preflight em adapter de mídia e materialização transportável limitada ao subconjunto suportado | `crates/core/src/scene_resources/`, `crates/media/src/directed_resources/`, testes próprios e exports mínimos, `docs/features/scene-resources.md`, relatório I | F038 |
| J | BL012/BL013/BL020/BL021 (BB07 inicial) | Árvore/seleção por ID, avaliação consultável e ciclo semântico de gesto com preview isolado/commit único/cancel; lib e JSONL, sem UI visual | `crates/core/src/scene_interaction/`, `scene_project/`, `ops.rs`, `state/mod.rs`, `state/history.rs` se necessários, testes próprios, `docs/features/scene-interaction.md`, relatório J | F039 |
| K | BL018 (BB10) | Oráculo ampliado com fronteiras JOIN, FPS racionais e matrizes mundo; consumidores Rust e controles negativos | `scripts/migration-oracle/`, novas fixtures/testes específicos sem editar módulos de produção, `docs/features/migration-oracle.md`, relatório K | F032 existente |

Exports de módulos em `core/src/lib.rs` e `media/src/lib.rs` são exclusivos do coordenador.
Não alterar Cargo.toml/lock sem necessidade demonstrada e mensagem ao coordenador. Não tocar runtime,
desktop ou projeto do usuário. Nenhuma frente habilita scene bundle como `tessel-scene@1` por atalho.
Sem commits/push/PR pelos agentes: entregam alterações no checkout compartilhado e relatório, coordenador integra numa entrega.
O coordenador é o único dono de core/lib.rs e media/lib.rs; agentes propõem exports no relatório.
Agentes não rodam Cargo nesta rodada compartilhada: o coordenador compila e valida o conjunto
após os módulos estarem completos. Formatação por arquivo e testes Node independentes podem rodar.
Não declarar Rust validado sem o gate posterior.
Matriz/índices/backlog são integrados pelo coordenador a partir das linhas propostas nos relatórios.

## Contratos entre H e I

- Recursos mantêm o `assetId` do bundle e o `relativePath` como origem declarada; nenhuma dependência
  de caminho absoluto no core. Identidade de asset não é seu caminho nem hash.
- H enumera referências necessárias ao frame com IDs explícitos. I resolve o manifesto a partir
  do bundle e permite consultar os mesmos IDs. Dados que o importador preserva mas não interpreta
  são declarados não suportados, nunca recursos silenciosamente perdidos.
- H usa `Bundle::evaluate_frame` e o grafo já existente; não reimplementa câmera/animação/matrizes.
- I não remove as guardas de pacote atuais antes da integração com o fluxo real e da prova sem origem.
- J reutiliza `scene_edit` e os gates; preview de gesto não modifica o documento/histórico persistente.
  Commit vira um lote atômico na pilha existente; cancel descarta candidato. Conflito de revisão/undo,
  troca de cena ou detach invalida gesto com erro tipado; consulta não cria entradas de histórico.
- K fixa expectativas pelo código real da origem somente leitura no snapshot d6adac15…;
  não muda expectativas só para fazer o Rust passar, nem incorpora o branch piloto.

## Cargo, GPU e disco

Toda chamada Cargo nesta rodada passa pela fila local:

```sh
python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-core --locked
```

O helper executa `./scripts/cargo` no worktree corrente, com lock de arquivo e
`MBX_GC_MAX_TOTAL_SIZE=64GiB`. O build release do app atual ocupa o primeiro slot.
Não trocar HOME/RUSTUP_HOME/CARGO_HOME. Não executar mbx clean/gc nem apagar target.
Verificar disco antes de gerar dados; piso 6 GiB, artefatos próprios até 2 GiB por frente.
Não rodar Chrome/GPU/provas de janela em paralelo: esta rodada é pura/oráculo/I/O pequeno.

## Aceite de integração

Cada frente entrega teste público, erro com invariância, doc/linha de matriz e atualizações dos
seus BLs (com provas por lib/headless/UI e limites). Não marcar BB05/BB06/BB07 completos pela
entrega só desta fatia. H/I entregam o contrato que destrava renderer/package; J não prova gesto
físico de ponteiro. Depois: gates conjuntos, renderer e prova mínima monitor/headless/pacote/export.
