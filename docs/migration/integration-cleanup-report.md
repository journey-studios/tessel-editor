# Integração BL089 — BL096, BL097, patch de host do R5 e BL095

Follow-up de integração da [R5](next-r5-report.md). Contrato e evidências por superfície:
[F040](../features/scene-resource-integration.md) ("Limpeza de resultados cancelados, falhos ou
descartados"). Estado e atribuição ficam só no [backlog](../backlog.md); este relatório propõe, não
edita fila nem matriz.

Estado: **implementado e testado** pela biblioteca e pelo `MediaHost` real. Cross-process, UI e
`job.discard` **não** fazem parte.

## O que mudou

| Item | Resultado | Onde |
|---|---|---|
| BL096 — arquivos compartilhados entre duas preparações/adoções | Registro de reivindicações por processo: só o último titular remove um arquivo criado por resultado pendente; aceitar entrega o arquivo ao projeto para todos os titulares | `scene_resources/claims.rs` (novo), `cleanup.rs`, `adopt.rs`, `import.rs` |
| BL097 — arquivo reescrito no lugar com mesmo tamanho/inode | Identidade por SHA-256 (o que `materialize` verificou) mais metadata antes/depois; falha fechada | `cleanup.rs` |
| Patch `next-r5-host.patch` | Aplicado **à mão** em `host.rs` (o patch não casava com o arquivo atual da R1); `SceneExport` da R1 intacto | `host.rs` |
| `AdoptionTarget` com workspace | Um adopt iniciado em outro workspace do mesmo projeto agora também é `stale_result` | `adopt.rs` |
| BL095 — `Journal` sem `Drop` | `ArmedJournal` desfaz os links ao desenrolar a pilha | `directed_resources/materialize.rs` |
| BL094 — `job.discard` | **Não feito** (explicitamente pendente) | — |

Arquivos tocados (todos sob `crates/media`): `src/scene_resources/{claims,cleanup,adopt,import,mod}.rs`,
`src/directed_resources/materialize.rs`, `src/host.rs`, e os testes `tests/scene_cleanup_ownership.rs`
(novo), `tests/scene_adopt_host.rs` (novo), `tests/scene_import_cleanup.rs` (21 → 29). Docs:
`features/scene-resource-integration.md` e este relatório. `lease.rs` **não** foi alterado.

Mudanças de API públicas de `scene_resources`: `CleanupReport.shared` (também em `to_json`),
`tracked_under(root)`, `AdoptionTarget.workspace`, `Created::{join_file, join_dir, relied_on}`;
`Adoption` e `PreparedSceneImport` perderam `PartialEq` (continuam sem `Clone`). O workspace
compila inteiro (`check --workspace --locked`).

## Como cada defeito foi reproduzido antes do conserto

Relatado como executado nesta sessão, antes de aplicar cada correção:

- **BL096 / BL097 (suíte de ownership).** Escrita primeiro contra o código sem registro e sem
  hash: 12 dos 17 testes então existentes falharam (descartar o primeiro apagava o arquivo do
  segundo aceito, em ambas as ordens; a reescrita no lugar de mesmo tamanho era apagada; a pasta
  `scenes/` de uma "outra cena" ficava). Os 6 restantes (e os acrescentados depois) cobrem
  limites e o que já funcionava.
- **Patch de host.** Com os dois hunks de `host.rs` revertidos (host sem `Pending::SceneAdopt`),
  **9 dos 13** testes de `scene_adopt_host` falharam; com o patch aplicado, 13/13.
- **BL095.** O teste de `panic` em `materialize_observed` falhava com a raiz suja antes do
  `ArmedJournal`.

Mutações feitas agora sobre o código final (arquivos restaurados e conferidos com `cmp`; rodadas
com `--no-fail-fast` em `scene_cleanup_ownership`, `scene_import_cleanup`, `scene_adopt_host`):

| Mutação | Falhas |
|---|---|
| M1: `leave` nunca responde `Shared` (o primeiro a sair remove) | 1 em `scene_adopt_host`, 10 em `scene_cleanup_ownership`, 5 em `scene_import_cleanup` |
| M2: o hash do conteúdo nunca reprova | 3 em `scene_cleanup_ownership` (mesmo tamanho; mtime restaurado; 1 byte); os demais passam, ou seja, o tamanho/inode sozinho já pega os casos que mudam o arquivo |

Achado honesto: o teste de estresse com threads (`results_sharing_a_folder_across_threads_*`)
**passa mesmo sob M1**, porque quem aceita primeiro já tira as entradas e os descartes seguintes
não encontram nada. Ele protege contra corrupção e vazamento sob concorrência, não prova a
exclusão mútua do descarte; a prova da regra são os testes determinísticos de ordem.

## Testes realmente executados (verdes, última rodada)

`cargo-serial.py test -p tessel-editor-media --locked --no-fail-fast`, nos binários:

| Binário | Resultado |
|---|---|
| lib (unitários, inclui 4 de `claims`) | 35 ok, 1 ignorado |
| `scene_cleanup_ownership` | 23 ok (+1 medição ignorada) |
| `scene_import_cleanup` | 29 ok |
| `scene_adopt_host` | 13 ok |
| `scene_resource_integration` | 19 ok |
| `scene_resource_publication` | 4 ok |
| `scene_project_guards` | 4 ok |
| `directed_resources` | 19 ok |
| `directed_resources_transport` | 20 ok |
| `interchange` | 2 ok |
| `package` | 4 ok |
| `package_atomic` | 10 ok |
| `import` | 4 ok |
| `scene_export` | 7 ok (confirma `SceneExport` da R1 e que `host.rs` compila) |
| `async_host` | 6 ok |

Repetição: `scene_cleanup_ownership`, `scene_import_cleanup` e `scene_adopt_host` rodadas 10 vezes
seguidas, sem nenhuma falha.
Também: `clippy -p tessel-editor-media --all-targets --locked -- -D warnings` limpo;
`fmt -p tessel-editor-media -- --check` sem diferenças; `check --workspace --locked` ok.

**Não executado:** `pnpm`/UI/desktop além do `check`, Windows, dois processos reais sobre o mesmo
workspace, escritor hostil concorrente, disco cheio, FFmpeg real fora das suítes acima,
medição em build de release. Nenhuma operação Git de escrita foi feita.

## Limites declarados

1. **Registro por processo.** Dois editores (ou o editor e a CLI) sobre o mesmo workspace só
   dividem o `RootLease`, que serializa mas não conta referências. O contrato de BL096 vale dentro
   de um processo.
2. **Mesmos bytes.** Um arquivo reescrito com o conteúdo idêntico não se distingue do publicado e
   é removido (nada que o escritor tenha escrito se perde).
3. **Janela residual.** Entre a última conferência e o `unlink` um escritor ainda vence;
   cooperativo, não é defesa contra escritor hostil do mesmo usuário (BL079 segue parcial).
4. **Custo do hash.** Descartar lê cada arquivo a remover. Medição em build de **debug**: um arquivo
   de 256 MiB levou 8,7 s (~29 MiB/s), por `measure_discarding_a_result_with_a_large_file`
   (`--ignored --nocapture`). Release não foi medido. Descartar resultado grande no destrutor da
   thread de UI bloqueia: a saída é BL094 (`job.discard`).
5. **Raiz ocupada.** Passados ~200 ms de espera pelo lease, os arquivos ficam e o relatório diz
   (`busy_roots`/`kept`); nunca se força.
6. **Unix.** Dispositivo/inode só em Unix; em outras plataformas valem tamanho + conteúdo (não
   testado). Windows e NFC/NFD seguem em BL078.
7. `undo` do import não remove arquivos; um job concluído e nunca confirmado depende de
   `job.commit` ou do descarte do host.

## Propostas de backlog (para o coordenador integrar)

| Item | Proposta |
|---|---|
| BL089 | Passa a **A validar** com os testes acima. O encerramento pode esperar a decisão sobre BL094 e sobre o limite cross-process |
| BL094 | Segue **Pendente**. Sem ele a limpeza de resultados grandes só ocorre em destrutor/commit |
| BL095 | **Feito e testado** (`ArmedJournal`; testes de `panic` em `materialize_observed` e em `adopt`); pode fechar |
| BL096 | **Resolvido** no escopo de um processo; referenciar o novo item cross-process |
| BL097 | **Resolvido** com identidade por conteúdo; limites 2 e 3 acima ficam registrados |
| Novo (R6x) | Reivindicações entre processos: exigiria um arquivo de reivindicações sob o `RootLease` (por pasta de cena) em vez do registro em memória. Só vale se houver o caso real de dois editores no mesmo workspace |
| Novo (opcional) | Mover a verificação de hash para fora da thread de UI (casa com BL094) e medir em release |
| F040 (matriz) | Linha da matriz: troca "patch de host pendente" por "integrado e testado com `MediaHost`"; remove "sem contagem de referências" e "`Journal` sem `Drop`" dos limites; mantém UI **P**, cross-process e Windows como não provados |

## Nota para quem integra

- O patch `next-r5-host.patch` foi aplicado manualmente: `scene_adopt` captura o
  `AdoptionTarget` antes de iniciar o job e devolve `Pending::SceneAdopt(Box<PendingAdoption>)`;
  `apply` chama `PendingAdoption::apply`. O resto de `host.rs` (R1) não foi editado.
- O arquivo do patch continua no repositório como referência; pode ser removido pelo coordenador.
- Antes de alterar o contrato do relatório (`shared` em `CleanupReport`), os consumidores do JSON de
  `scene.resources.adopt` só veem um campo novo e aditivo.
