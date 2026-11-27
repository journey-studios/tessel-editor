# W1 — portabilidade Windows e geometria (BL167.3, BL167.7)

Relatório da frente W1 da rodada Windows de 14/11/2026. O [backlog comum](../backlog.md) continua sendo o
único registro de execução: este arquivo guarda causa, reprodução e evidência. As linhas do backlog e da matriz
são **propostas ao coordenador** (seção final); nenhum arquivo comum foi editado.

## Ambiente e identidade

| Item | Valor |
|---|---|
| SO | Windows 10.0.26200 (x64), shell PowerShell; Git Bash (`C:\Program Files\Git\bin\bash.exe`) executa `scripts/cargo`, `scripts/runtime` |
| Checkout | `E:\www\tessel-roadmap-w1-20261114` (preparado pelo T3; nenhum outro checkout criado ou tocado) |
| Origem / base | `https://github.com/journey-studios/tessel-editor.git`; base `2b54ac13b4b9f595b14614ae541cd0beb5921daf` (`origin/main` após `git fetch`; PR57); árvore limpa no início |
| Branch | `w1/windows-portability-bl167-3-7` |
| Recursos antes de editar | disco E: 336 GB livres (304 GB no fim); RAM 32,8 GB livres de 65,3 GB |
| Ferramentas | Rust 1.99.1, `scripts/cargo` → mbx 1.29.x, target padrão (sem `CARGO_TARGET_DIR`/`--target-dir`/lane, sem `clean`/`gc`); FFmpeg 9.0.2 (WinGet) por `TESSEL_FFMPEG`; Node e Chrome por descoberta (BL167.2); `TESSEL_REQUIRE_RUNTIME=1` nas provas de render; `./scripts/runtime setup` executado neste checkout (`runtime/dist`, 72 MB de `node_modules`, fora do Git) |
| `shasum` | ausente no Windows (`Get-Command shasum` vazio, nem em `Git\usr\bin`) |
| Binário desktop do commit final | `target\debug\tessel-editor.exe`, 69 352 960 bytes, SHA-256 `4BC94C1084CBBCC516CE0EAC44AFF8CD8A898F9EDF442EF7943C65BDD44F25FA` (build debug do código final, sem janela aberta) |
| Logs | `C:\Users\wally\tessel-readiness-logs\2026-11-14-w1\` (1,6 MB; descartáveis grandes não existem) |
| GPU / desempenho | nenhum lock de GPU tomado; nada aqui é medição de desempenho, GPU, ponteiro físico ou áudio ouvido |

## BL167.3 — `shasum` nos testes de render

**Causa.** `crates/render/tests/scene_export.rs` chamava `Command::new("shasum")` em
`declared_audio_is_mixed_video_only_is_explicit_and_unevaluated_domains_are_refused` e
`the_headless_binary_mixes_the_declared_wav_at_the_requested_range`. `shasum` é um script Perl de macOS/Linux.

**Reprodução (antes, `07-render-before.log`).** Com o runtime construído e `TESSEL_REQUIRE_RUNTIME=1`:
`cargo test -p tessel-editor-render --test scene_export -- <os dois>` → exit 101, os dois falham em
`scene_export.rs:605` e `:835` com `Error { kind: NotFound, message: "program not found" }`. Sem o runtime os testes
se auto-ignoram: o `shasum` só aparece com o runtime montado.

**Correção.** Os dois pontos usam o hash já existente do produto, `tessel_editor_media::fsx::hash_file` (SHA-256 em
streaming), por um helper local `sha256(&Path)`, o mesmo que `scene_splat_frames.rs` já usava. Nenhuma asserção mudou:
o hash continua sendo o `sourcePcmHashSha256` que o produto verifica.

**Depois (`08-render-after.log`).** `scene_export`: 12/12, exit 0, incluindo os dois. Uma repetição intermediária
falhou nos mesmos dois pontos porque o arquivo, restaurado com `Copy-Item` depois do controle "antes", tinha mtime antigo
e o cargo reutilizou o binário do código velho (o log foi sobrescrito pela repetição válida, feita após atualizar o mtime).

## BL167.3 — gizmo: duas falhas, duas causas diferentes

Reproduzidas em `2b54ac1` (`01-gizmo-before.log`): `scene_gizmo_rotation` 45/46, `scene_gizmo_scale` 53/54.

### Rotação — defeito de produto (fecho do anel)

`the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them` exige `points.first() == points.last()`.
Falhava com `v` `0.6491406061697664` × `0.6491406061697665`.

**Causa demonstrada.** `Layout::rings_at` calculava o último ponto com o ângulo `TAU * 64 / 64`. `TAU` é o `f64`
mais próximo de 2π, não 2π: um programa isolado (`rustc`, descartado) imprime `cos(TAU) = 1` e
`sin(TAU) = -2.4492935982947064e-16` (`0xbcb1a62633145c07`). Esse resíduo entra em `origin + offset·raio` e na
projeção; se ele sobrevive ao arredondamento depende da vista e da libm. O polígono "fechava" por coincidência:
o contrato (65 pontos, o último é o primeiro) não era garantido.

**Correção (`gizmo/rotation.rs`).** O último ponto usa o ângulo `0`. Nenhum outro ponto muda; o último muda de no
máximo `2,5e-16` (o resíduo eliminado).

**Regressão discriminante.** `every_ring_closes_on_its_first_point_bit_for_bit_in_any_view_space_and_pose`:
4 câmeras × 4 poses × `world`/`local` × 3 anéis = 96 anéis, igualdade bit a bit do primeiro e do último. No código
anterior ela falha (`02-rot-regression-prefix.log`, anel `x`, câmera `[0,0,8]`, `local`, pose de 40° em `[1,2,0.5]`);
depois, `scene_gizmo_rotation` 47/47 (`03-rot-after.log`).

### Escala — não é erro do gizmo nem da geometria do plano

`four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear` falhava em
`the plan agrees with the oracle chain`: `[1][1]` `-4.44e-8` (plano) × `-2.22e-8` (oráculo).

**Diagnóstico.** O plano não compõe o quaternion armazenado. `Quat::to_euler_xyz_degrees` →
`Quat::from_euler_xyz_degrees` (o desvio do Three.js, documentado em `directed_scene/transform.rs`) usa
`asin`, `atan2`, `sin` e `cos`. Para o quarto de volta `(0, 0, s, c)`, `s = c = 0.7071067811865476`, o desvio devolve
Euler `z = 90.00000000000001` e o quaternion `(0, 0, 0.7071067811865477, 0.7071067811865474)`: `z` 1 ulp acima,
`w` 2 ulp abaixo. Com quatro pais de `(0,01; 100; 100)` as linhas da matriz de mundo diferem em 16 ordens de grandeza e esse
ruído de `~1e-16` vira `~1e-8` numa entrada que vale 0 no papel. **Prova (diagnóstico descartável em
`04-scale-diag.log`, removido do arquivo):** a cadeia do oráculo do teste, `Π(Rᵢ·Sᵢ)` escrita no próprio arquivo,
alimentada com o quaternion devolvido pelo desvio, é **igual ao plano bit a bit** (`plan==oracle(q') bits: true`); com
o quaternion armazenado ela difere de 1–2 ulp. A asserção antiga (`1e-9` da maior entrada da linha, ou `1e-13`) pede o
mesmo ruído bit a bit; passava no Mac onde o desvio devolve o mesmo quaternion (a confirmar, ver Mac).

**Decisão.** Sem mudança de produto: o desvio é o comportamento de equivalência com a origem, e o plano é correto
dentro do limite. Sem relaxar tolerância arbitrária: o limite é derivado.

- `chain_error_bound` = erro entrada a entrada de um produto de `n` fatores `Aᵢ = Rᵢ·Sᵢ` com cada `R` errada de
  `δ = 16·EPSILON`, na **primeira ordem**: `Σᵢ |A₁|…|Aᵢ₋₁|·(δ·J·|Sᵢ|)·|Aᵢ₊₁|…|Aₙ|` (`J` de uns) mais o arredondamento do
  produto, `Π|Aᵢ|`. A entrada `[1][1]` fica admitida; uma entrada de `1e-4` num produto de `1e8` continua recusando
  erros reais.
- Teste do juiz, `the_chain_error_bound_admits_last_place_noise_and_refuses_any_real_difference`: aceita 1–2 ulp por
  componente do quaternion (três combinações) e **recusa** um quarto de volta errado em `1e-9 rad`, os dois primeiros
  elos trocados, o giro oposto e uma escala errada em `1e-12`.
- A asserção `assert_matrix_by_rows` (`1e-9` relativo à linha) continua nas demais; elas passam e não foram tocadas.

Depois: `scene_gizmo_scale` 55/55 (`05-scale-after.log`).

### Não alterado e a registrar

O anel da luz pontual (`gizmo/light.rs:171-175`) tem o mesmo padrão `TAU * step / RING_SEGMENTS` e **não foi alterado**
(fora do ownership e sem teste que falhe hoje no Windows). Seu fecho continua dependente da plataforma: proposta
BL167.3.1.

## BL167.7 — `tree()` de `directed_resources_common`

**Reprodução (`09-transport-before.log`).** `directed_resources_transport`: 13 passam, 3 falham, exit 101:
`files_land_at_their_declared_paths_and_the_source_is_untouched`,
`splats_and_the_audio_mix_are_carried_too` e
`a_different_file_already_at_the_destination_is_a_conflict_and_is_never_overwritten`; esquerda
`["assets", "assets\\backdrop.png", …]`, direita `["assets", "assets/backdrop.png", …]`.

**Causa.** `tree()` escrevia o caminho relativo com `Path::display()` (`\` no Windows), e as expectativas são os
caminhos relativos declarados do bundle (`/`). O produto está correto: ele escreveu os arquivos nos lugares certos.

**Correção.** Só a representação do helper: os componentes do caminho relativo são unidos por `/` (`components()`,
`join("/")`). A ordenação roda depois da normalização, então a ordem é a do Unix. Nada é adicionado, escondido ou
tolerado; nenhuma expectativa mudou; no Unix `components().join("/")` é igual ao `display()` anterior.

**Testes.** Novo `the_tree_helper_lists_nested_entries_with_slashes_in_the_order_unix_gives`: subpastas, pasta
vazia e o irmão `a0`, que ordena entre `a/b` e `a\b` (a ordem só é a do Unix se normalizar antes de ordenar); entrada
extra e entrada ausente mudam a lista (nada é tolerado); nenhuma barra invertida sai. Os três testes
originais e os conflitos continuam com listas exatas.

**Depois (`10-transport-after.log`).** `directed_resources_transport` 17 passam, 0 falham (13 + 3 + 1 novo; o módulo
`containment` só-Unix não roda no Windows: 20 + 1 = 21 no Mac, **pendente**); `directed_resources` 14/14.

**Outros alvos que incluem o helper** (`12-after-*.log`, `--no-fail-fast`, 240 s por alvo, nenhum estourou):

| Alvo | Resultado |
|---|---|
| `scene_resource_integration` | 19/19 (o levantamento de 13/11 registrava 1 falha) |
| `scene_resource_publication` | 3/4 (13/11: 2 falhas) |
| `scene_adopt_host`, `job_discard`, `job_discard_exit`, `scene_attach_job`, `scene_splat_package`, `scene_claims_jsonl`, `scene_cleanup_ownership` | todos ok |
| `scene_import_cleanup` | 31/32: **nova causa na mesma classe** (abaixo) |
| `scene_claims_failure` 5/6, `job_discard_claims` 6/13, `scene_claims_integrity` 7/11 | falhas de ledger/claims (`Acesso negado`, caminho `\\?\`, lock): **W2/BL167.6**, não representação de caminho |

As contagens de 13/11 são do relatório BL167.1 (outra árvore e outra hora): servem de contexto, não de A/B
controlado. Os alvos `job_discard_claims`, `scene_claims_integrity`, `scene_claims_failure`,
`scene_claims_processes` e o teste de lib que trava **não são afetados por esta entrega** e ficam com W2.

**Fora do ownership, proposta (BL167.7.1).** `crates/media/tests/scene_import_cleanup.rs:180-185` define um closure
`names` que também usa `display()`: `an_adoption_records_the_files_and_folders_it_made_and_nothing_on_a_reuse` falha com
`["scenes", "scenes\\street", "scenes\\street\\assets"]` × `["scenes", "scenes/street", …]`. É o mesmo remédio
(`components().map(…).join("/")`), uma linha; não apliquei para não colidir com W2.

## Gates (código final, Windows)

Todos com a árvore deste commit, `./scripts/cargo` (Git Bash), target padrão:

| Comando | Resultado | Log |
|---|---|---|
| `cargo fmt --all -- --check` | exit 0 (depois de `cargo fmt` em 3 trechos de testes meus) | `20-fmt.log` |
| `./scripts/cargo check --workspace --locked` | `Finished`, sem erro | `21-check.log` |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | `Finished`, sem aviso | `22-clippy.log` |
| `./scripts/cargo test --workspace --locked --no-fail-fast --exclude tessel-editor-media` | exit 0, 89 alvos, **2106 passam, 0 falham, 10 ignorados** em 432 s; render com runtime real | `23-test-workspace-ex-media.log` |
| `./scripts/cargo build -p tessel-editor --locked` | exit 0 | `24-build-desktop.log` |
| `cargo tree -p tessel-editor-core` | 0 linhas com `gpui`/`wry` | (saída vazia) |

**Limite honesto.** O workspace inteiro **não** foi executado: `tessel-editor-media` foi excluído porque o teste de
lib `emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` trava (W2/BL167.6) e esta frente não pode esperar
por ele nem pulá-lo. Em vez disso rodei os alvos de integração de `media` listados acima, com prazo. O que não
roda: a lib de `media` inteira e `scene_claims_processes`, `thumbnails`, `scene_claims_*` de W2.

## Superfícies

| Superfície | Evidência |
|---|---|
| Biblioteca (core, sem GPUI) | `scene_gizmo_rotation` 47/47 e `scene_gizmo_scale` 55/55 em processo; `headless::execute` |
| CLI headless real | binário `tessel-headless` (`target\debug`), sessão JSONL com `project.create`, `scene.attach`, `scene.select`, `scene.gizmo.tool` e `scene.gizmo.state`: `ok`, anéis `x`, `y`, `z` com 65 pontos e primeiro == último bit a bit |
| UI (janela) | **A validar.** Nenhum controle, `Command`, nó de árvore ou parser mudou; o overlay é o mesmo contrato com o último ponto exato. Não rodei `./scripts/ui` nem alego observação de janela |
| Matriz de capacidades | sem mudança de linha (sem feature nova); F055 ganhou a seção de portabilidade |

## Mac — pendente do coordenador

- `scene_gizmo_rotation` e `scene_gizmo_scale` (esperado: passam; o quaternion devolvido pelo desvio de Euler é lá
  exato ou dentro do limite; **se o limite do Mac divergir, não relaxar: reportar**).
- Fecho do anel: o novo teste precisa passar também lá (a hipótese é que o resíduo `-2,4e-16` era absorvido lá).
- `directed_resources_transport` 21 (com o módulo Unix de `containment`), `directed_resources` 19.
- `render --test scene_export` 12/12 com runtime e `shasum` fora do caminho.
- Libm: não afirmo nada sobre libm do Mac sem rodar; a causa do anel é aritmética de `f64`, a da escala é o desvio de Euler
  com `asin`/`atan2`/`sin`/`cos`.

## Negativos, rollback e o que ficou

- Rollback: `git revert` do commit desta entrega desfaz tudo; só o `rotation.rs` é código de produto (3 linhas úteis).
- Nada foi pulado, nem `cfg`, nem `#[ignore]`, nem asserção removida; nenhuma tolerância das demais asserções mudou.
- Não alegado: desempenho, cache, GPU, ponteiro físico, áudio ouvido, janela.
- Gap registrado: anel da luz (BL167.3.1), `scene_import_cleanup` (BL167.7.1), Linux nativo não executado.

## Propostas ao coordenador (backlog e matriz)

Não editei `docs/backlog.md`, `docs/features/README.md` nem `docs/README.md`.

```
| BL167.3 | Windows B4/B5 | … | Concluído Windows — Mac a validar | BL167 | W1 | Fonte: commit desta entrega na branch `w1/windows-portability-bl167-3-7`, base 2b54ac1. Render: hash_file do produto, 12/12 com runtime real (antes 2 falhas `program not found`). Rotação: fecho do anel por construção (TAU tem sin = -2,4e-16), 47/47, regressão 96 anéis falhava antes. Escala: causa = desvio quaternion→Euler→quaternion do plano (bit a bit igual ao oráculo com o quaternion devolvido); limite derivado + teste que recusa 1e-9 rad, 55/55. Hipótese de libm não usada. [relatório](migration/windows-w1-portability-report.md), [F055](features/scene-gizmo.md#portabilidade-windows-bl1673-w1) |
| BL167.3.1 | Gizmo / luz | Anel da luz pontual repete `TAU * step / RING_SEGMENTS` (`gizmo/light.rs`) | Pendente | BL167.3 | — | Mesmo fecho por construção e teste bit a bit do anel da luz; hoje depende da plataforma |
| BL167.7 | Windows / fixture media | … | Concluído Windows — Mac a validar | BL078 | W1 | tree() une componentes por `/`, ordena depois; `directed_resources_transport` 17/17 (antes 13 + 3 falhas), +1 teste do helper; Unix 21 pendente no Mac |
| BL167.7.1 | Windows / fixture media | `scene_import_cleanup` usa `display()` no closure `names` | Pendente | BL167.7 | — | 1 falha; mesma correção, fora do ownership de W1; verificar com W2 |
```
