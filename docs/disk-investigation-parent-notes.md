# Investigação complementar de disco — somente leitura

2026-11-04, medição do pai durante a investigação do Cursor. Nada apagado, movido,
desmontado ou encerrado. Data tinha 2,6 GiB livres no início desta leitura.
Os valores são blocos alocados reportados por du/stat, não garantia de recuperação
física no APFS (hardlinks, clones e arquivos abertos podem compartilhar/reter blocos).

## Achados

- `~/Library/Caches/mbx`: 10.07 GiB por `du -sk`; actions 3.44 GiB, targets 5.86 GiB,
  incremental 0.74 GiB. O `target` do tessel-editor é symlink para targets/v1 deste cache:
  **não somar target novamente**. Não recomendar apagar todo cache durante builds.
- `/private/tmp/tessel-base-426693`: 3.66 GiB, dos quais 3.46 GiB em node_modules.
  É worktree/cópia Tessel com `.git`, apps e packages, **não um target Rust descartável**.
  `lsof` mostrou módulos sharp/libvips usados por processos Node 20478, 69378, 85993, 85994.
  Preservar; verificar dono da tarefa antes de qualquer limpeza.
- `/var/folders/h2/6xxl56j97cnfzthk1mgmpz540000gn/T/tessel-media-export-disk-*`:
  16 diretórios somam 1130.8 MiB alocados. Os maiores são `small.dmg` de 100 MiB;
  exemplos sufixos 96778, 95697, 92463, 87775, 86867, 85690, 83161, 68801.
  Esses PIDs não apareceram em `ps`. Nenhum mount tessel/export/small apareceu em `mount`.
  Candidatos estreitos para remoção após aprovação/verificação final de uso.
  Código de origem: `crates/media/tests/export.rs`, teste
  `a_full_disk_ends_the_export_with_disk_full_and_leaves_the_volume_clean` cria `small.dmg`,
  desmonta no fim, mas não remove o diretório/imagem. A versão atual pede 40m; arquivos
  antigos medidos tinham 100 MiB. Repetir a suíte acumula esses artefatos.
- `/private/tmp/tessel-home`: 596.7 MiB, com .rustup 463.1 MiB e .cargo 55.9 MiB.
  Parece HOME isolado de testes; confirmar symlinks/uso e origem antes de remover.
- `/private/tmp/tessel-seek`: 130.5 MiB, dos quais 129.8 MiB no binário antigo em old/.
  O relatório seek-performance declara instâncias próprias encerradas; preservar runs/
  e relatório e confirmar uso atual antes de remover o binário.
- `/private/tmp/tvb`: 344.6 MiB; fixtures e provas iniciais Opus. Dois splats somam
  122.1 MiB, PNGs ~207 MiB. Preservar resultados/harness antes de limpar fixtures;
  o parecer anterior dependia de dados temporários, ainda podem ser necessários.
- `/private/tmp/tev`: 1.165 GiB nesta leitura. Splats 283 MiB, PNGs 237 MiB,
  MP4s 145 MiB, WebMs 68 MiB, binários sem extensão 316 MiB. **Ativo:** PID 49971
  tessel-render, ffmpeg 54943 escrevendo exp-sw/.tessel-staging-export-49971-6,
  Node/Chrome e cadeia raw/chain3.log. Não apagar enquanto a prova estiver ativa.

## Prioridades

1. Imagens temporárias dos testes de disco cheio: potencial ~1.1 GiB, verificar todos
   os handles/mounts no instante da limpeza e preservar outros testes/projetos.
2. Binário antigo de seek e HOME isolado, só após confirmar inatividade/proveniência.
3. Fixtures Opus apenas após consolidar evidências e encerrar validação.
4. Cache mbx exige orçamento/GC gerenciado e coordenação com builds, não rm em target.

Vídeos não são a principal causa neste recorte. Caches de compilação e resíduos de
testes são maiores. Não extrapolar estes achados para apagar projetos/worktrees.

Prevenção proposta: cleanup por RAII/temporários nos testes de imagem de disco, inclusive
em panic; orçamento e cleanup de fixtures próprias ao encerrar suites; separar binário
para teste de diretórios gerenciados pelo GC. Não houve implementação dessas medidas aqui.
