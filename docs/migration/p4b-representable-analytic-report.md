# P4b — X estendido representável: relatório (BL082.4.17)

Plano pré-registrado: [p4b-representable-analytic-plan.md](p4b-representable-analytic-plan.md) (escrito antes de qualquer código).
Contrato: [F064](../features/fx-transitions.md). Backlog único: [docs/backlog.md](../backlog.md) (**não editado aqui**; o patch literal
está na seção 9, para o coordenador). Ramo `feat/p4b-representable-x`, base `main` `b6cf304`.

## 1. Estado honesto

| Item | Estado |
|---|---|
| Oráculo analítico com 7 cenários **válidos** (a, b, b2, c, d + extra x + controle ctl), 98 quadros, os dois decks em todos | **Entregue; a validar na revisão** |
| Bundle de destino completo por cenário, importado pelo core real de `main` (`Accept`, só testes) | **Importa; limites, `held`, câmera, marcador e luz conferem** |
| Portão público (`import_value` com transição) | **Continua `FX_TRANSITION_NOT_YET`** |
| Pixel real, avaliação de dois decks, hold fora do shot avaliado pelo core, `ScenePlan.transition` | **Pendentes (S1b/S2)**: nenhum consumidor validou estes dados |
| Paridade com os 59 casos de F064, aceite humano, motor | **Pendentes**; `ORIGIN_NOT_EXECUTED` e `ORIGIN_GOLDEN_NOT_COMPARABLE` valem |

`origin-formula` sobre cenas sintéticas, `destination-validity-checked`. Não é golden executado, a origem não aceitou estes
bundles (alfa opaco, política do destino) e **um oráculo pronto não é S2-validado**: T06/T10/X só podem alegar "pixel de A com hold"
nestes cenários depois de um consumidor real (seção 8). Os três `lit-*-held` antigos **não foram tocados** e continuam sem essa
alegação.

## 2. O que mudou em relação à S0

* Instância **e** câmera cobrem o próprio shot em todos os shots (guardas `REP_SHOT_EXCEEDS_INSTANCE`, `REP_CAMERA_SHORT`); o hold
  só existe no deck **vizinho fora do seu intervalo**. Isto evita a divergência legada do plano primário com câmera curta
  (**BL082.4.23**, descoberta da S1b; não alterada aqui) e a recusa `SHOT_EXCEEDS_INSTANCE_DURATION` do importador.
* Os mutantes de relógio (`PIN SWP PRI UNH LIN LSR LCC LCE LNF`) valem **só para o vizinho**; o primário é a verdade dos dois lados
  (o gerador recusa com `REP_ISOLATION_BROKEN` se uma hipótese alterar o primário; a spec injeta o vazamento e vê a recusa).
* Sonda = par (quadro, pixel) distinto; um pixel com dois papéis é uma sonda só. Marcadores não planos entram em `skipped_probes`
  (nunca trocados por pixel próximo).
* Cores base do kit `lit` novas (pixel iluminado ≈ 0,9 linear no nível 1): a variação do flicker passa a separar ≥ 4 códigos.
* Quadros gravados: os 12 da janela mais o antes e o depois (14 por cenário), estados dos dois decks, segundos racionais.

## 3. Cenários e o que cada um prova

| Cenário | Kit | Alinh. / corte | Vizinho preso (instância, câmera) | Isolamento alegado |
|---|---|---|---|---|
| `rep-a-tail-both-held-light` | lit | start `[284,296)`, C 284 | A cauda: **all**, **all** (493 e 402) | relógio da luz do vizinho: `LIN`, `LCC`, `LCE` |
| `rep-b-tail-instance-held-camera-runs` | pos | start `[30,42)` | A cauda: **all**, **none** (instância 33, câmera 30..41) | `PIN` (câmera), `SWP`, `PRI`, `UNH` |
| `rep-b2-tail-instance-edge-camera-runs` | pos | start `[30,42)` | A cauda: **some** (real até o quadro 34, presa do 35 em diante), **none** | idem + o limite do flag `held` (instância presa a partir do quadro 35; `FLG`) |
| `rep-c-head-both-held-light` | lit | end `[463,475)`, C 475 | B cabeça: **all**, **all** (0, 0) | `LIN`, `LCC`, `LCE`, **`LSR`** |
| `rep-d-ntsc-tail-held-light` | lit | start `[1800,1812)`, 30000/1001 | A cauda: **all**, **all** (1799) | `LNF` (fps nominal), `LIN`, `LCC`, `LCE` |
| `rep-x-center-both-mixed` (extra) | both | center `[24,36)` | misto: **some**, **some** | só posição (`PIN`, `SWP`, `PRI`); luz **não** alegada |
| `rep-ctl-real-both` (controle, `handles use`) | both | center `[24,36)` | nada preso | `PIN` e `UNH` concordam; `SWP`, `PRI` rejeitam |

## 4. Matriz de hipóteses (gerada: `pilot_dissolve_representable.py matrix`)

`R:n/m` rejeita (n sondas em m quadros; limiar ≥ 3 sondas em ≥ 2 quadros e separação ≥ 4 códigos); `a` concorda (≤ 1 código em toda
sonda); `p` parcial; `FLG` mostra `R:` quadros com flag diferente.

| Cenário | PIN | SWP | PRI | UNH | FLG | LIN | LSR | LCC | LCE | LNF | DSW | NOH |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| a | a | a | a | a | R:2 | **R:28/7** | a | **R:28/7** | **R:28/7** | a | R:46/12 | R:5/4 |
| b | **R:16/9** | **R:15/8** | **R:8/8** | **R:17/9** | R:2 | a | a | a | a | a | R:68/12 | R:28/6 |
| b2 | **R:7/4** | **R:16/8** | **R:9/9** | **R:8/4** | R:2 | a | a | a | a | a | R:59/12 | R:25/6 |
| c | a | a | a | a | R:2 | **R:18/5** | **R:16/4** | **R:18/5** | **R:17/5** | a | R:48/12 | R:8/3 |
| d | a | a | a | a | R:2 | **R:25/7** | a | **R:25/7** | **R:25/7** | **R:20/5** | R:48/12 | R:8/3 |
| x | **R:5/3** | **R:9/4** | **R:14/4** | a | R:2 | a | a | p:2/1 | a | a | R:64/12 | R:13/4 |
| ctl | a | **R:8/5** | **R:15/5** | a | a | p:3/1 | p:0/0 | p:3/1 | a | a | R:55/12 | R:10/4 |

Em **negrito**, o que o cenário alega (todas as alegações foram atingidas; o gerador recusa o contrário).

**Variantes não discriminantes, reportadas (nada escondido):**

* `a`: `LSR` (A começa em 0: relógio do shot = relógio da edição) e `LNF` (24/1: fps nominal = real) **concordam** por construção;
  `PIN`, `SWP`, `PRI`, `UNH` concordam porque a cena é estática e as duas pernas de A terminam exatamente no corte.
* `b`, `b2`: nenhum cenário de luz; `LIN…LNF` concordam. `NOH` e `DSW` (nível da mistura) rejeitam em todos.
* `c`: `LNF` concorda (24/1). Em `c`, `LSR` **rejeita** (B começa em 475) enquanto em `a` e `d` concorda: os dois são complementares.
* `x` (centro, peso baixo nas pontas): luz **não** isola (`LCC` parcial 2 sondas em 1 quadro; `LIN`, `LSR`, `LCE`, `LNF` concordam). `UNH`
  concorda (nenhuma cauda de instância chega ao último quadro). Nenhuma alegação de luz neste cenário.
* `ctl` (controle): `LIN` e `LCC` ficaram **parciais** (3 sondas em 1 quadro): a alegação `LIN` do rascunho foi **retirada** depois da
  primeira execução (emenda registrada no plano; limiares intactos). `PIN` e `UNH` concordam (nada preso).
* `FLG` age sobre os estados de todo deck gravado (inclusive o primário no quadro antes/depois da janela, onde estão o último e o
  primeiro quadro reais): não é um relógio do vizinho.

## 5. Evidência (cada gate com HEAD e árvore)

Interpretador: Python **3.13.17** (`%LOCALAPPDATA%\Programs\Python\Python313\python.exe`, tags/v3.13.17:64e080d, MSC v.1944 64 bit),
sha256 `7c2fc2b303a4ee3cc14cc17b47bc6e8bca98c78792edd123b846a560da44b203`; o atalho da Store é recusado pelo gerador e pela spec
(exit 5), sem `SKIP`.

| Gate | Resultado | HEAD / árvore |
|---|---|---|
| `pilot_dissolve_representable.spec.py` (37 testes) | OK, normal e `-O` | `d1373f4`; árvore **limpa** após o commit; quando executada os dois arquivos novos de teste estavam não rastreados e idênticos aos commitados |
| `generate --check` | 14 arquivos byte a byte, exit 0 (também com `--origin`) | idem |
| `verify` offline / `--origin <repo>` | exit 0 ("pin NOT verified (attestation only); the origin was NOT executed") | idem |
| `verify --origin --require-pin` | **exit 4** (`PIN_UNVERIFIED`): o commit `d40ac3f7c0` **não existe** neste clone da origem; não declarado verificado | idem |
| `cargo test -p tessel-editor-core --locked --test pilot_dissolve_representable_import` | 5 testes OK | idem |
| `cargo fmt --all -- --check` | rc 0 (um `cargo fmt --all` antes formatou **só** o teste novo) | idem |
| `cargo clippy -p tessel-editor-core --all-targets --locked -- -D warnings` | rc 0 | idem |
| `cargo tree -p tessel-editor-core --locked` | 0 linhas com `gpui`/`wry` | idem |

Fora do escopo e **não executados**: build do app/node/motor, `--workspace`, base de gizmo/lease/adopt. Não se marca `Env` com
tampa 14 (a base tem 13). Todo `cargo` foi pelo wrapper `./scripts/cargo` no Git Bash real (`C:\Program Files\Git\bin\bash.exe`),
`target`/mbx padrão, sem `CARGO_TARGET_DIR`/`--target-dir`/`TESSEL_CARGO_LANE`/`clean`/`gc`. O próprio wrapper imprimiu
uma mensagem de "abandoned target directories removed": é a coleta automática do mbx do wrapper, não um comando meu
(o volume não foi registrado).
O `%TEMP%` tem 34 GB de terceiros; os meus artefatos são `%TEMP%\tessel-representable-x\` (baseline de hashes e o script
`gates.sh`), alguns KB. Nenhuma medição de desempenho, nenhum lock de GPU.

### O que o teste Rust prova e o que não prova

Prova, no core real de `main`: cada bundle **importa** sob `import_value_admitting(.., Accept)`; a importação pública continua
recusando com `FX_TRANSITION_NOT_YET` (e não por forma); o **controle negativo** (instância de 12 sob um shot de 30) continua
recusado com `SHOT_EXCEEDS_INSTANCE_DURATION`; instância e câmera cobrem o shot nos números importados; o tempo racional de edição,
o mapeamento dentro do shot, a pose da câmera e a posição do marcador (canal `event`) em cada quadro gravado, e a intensidade efetiva
da luz em `f / fps` (incluindo 30000/1001) conferem com o oráculo; e o flicker do **próprio core** separa o relógio da instância do
real em ≥ 3 quadros presos dos cenários que alegam `LIN`. **Não prova:** avaliação de dois decks, nenhum hold fora do shot pelo
core, `ScenePlan.transition`, nenhum pixel.

## 6. Proveniência, hashes e o que não foi tocado

Antes e depois: Windows NT 10.0.26200, cwd `E:\www\tessel-p4b-representable-x-20261114`, remoto `origin` =
`https://github.com/journey-studios/tessel-editor.git`, ramo `feat/p4b-representable-x`, base `b6cf304783c8a849d45b0a9144b0f406b99e16ef`
(`origin/main` **continua** `b6cf304` depois de `git fetch`; nenhuma integração, rebase, force ou reset), disco `E:` com ≈ 241,8 GB livres.
`crates/core/src` e `runtime` sem mudança (árvores `2223c77f…` e `c9358824…` do baseline; `git diff --name-status b6cf304 HEAD` só tem
`A`: arquivos novos). A origem `E:\www\tesselapp` tem **151** entradas sujas antes e depois: só `git rev-parse`/`cat-file` dos blobs
fixados (nada modificado, instalado ou executado).

**Originais, SHA-256 antes = depois (iguais):**

| Arquivo | SHA-256 |
|---|---|
| `docs/migration/fixtures/pilot-fx/dissolve-cases.json` | `67d3314f66aa0512666e93e515de0d66393298489168999172d7e37123d393d9` |
| `docs/migration/fixtures/pilot-fx/dissolve-manifest.json` | `77c4d7bc5f7b95adc37d2ec1560adffb7772c3aa6ee805079ffdb9f0a392ce42` |
| `scripts/pilot_dissolve_analytic.py` | `4abc1dbf01bba0067188c9c344ae48ba39e992e6307296d7029d67376068647a` |
| `scripts/pilot_dissolve_analytic.spec.py` | `5f1c360d86d4cad99218b9b5585e1d40307994a5d169f9297f5f52275068580a` |
| `scripts/pilot-dissolve-corpus.py` | `4762a10d6bce0182c6f31f4fba68175dbe0b21c016e231e49be89fb1d5bbf76e` |
| `scripts/pilot-dissolve-corpus.spec.py` | `502ca93d149bf9ad2f55862acb265e20894ace67bbb84b535467e6bc9891c51a` |
| `docs/backlog.md` | `9065962ab944116cb5fc42e262858886819ab1ea7f15dc02d378a1e7d50774eb` |
| `docs/features/README.md` | `0eb34c4675f7fa19d2ec6b972c4e1cff21ea58555c8abfe39bf64a4421771452` |
| `docs/README.md` | `32da639c36c04db4a20518ec00c33831868243dea7424e66131cb98aefb1d03d` |
| `docs/features/fx-transitions.md` | `54ecd09dee52dd1b1090b601077bb2dc50ad8a36819f5b6d96309dd14200f4f4` |
| `docs/migration/p4b-cross-dissolve-plan.md` | `62d26d083fc9aa26aa467286a7a82304a300fe8052d31b2db9aba1bd1d3d11b4` |
| `Cargo.lock` | `2ca1ab56e466213fe8203b0cab6ec0dd12060fd5967d786de787734c1dd7db63` |

**Arquivos novos (SHA-256 da entrega inicial publicada em `93246d1`; o ID local anterior `d1373f4` foi reescrito antes do push):**

| Arquivo | SHA-256 |
|---|---|
| `scripts/pilot_dissolve_representable.py` | `3121c207971a1e353c76ea24f679bb49aba5a7161cee2ee9bf453a857d86c290` |
| `scripts/pilot_dissolve_representable.spec.py` | `1ccdf1ef639dd65f835e6ad7ef352de51c3c2f6ef8be75ef947507dbd41b0dfe` |
| `docs/migration/fixtures/pilot-fx/dissolve-representable-cases.json` | `cf220e9cfa6e7fbc14f070f85376aee3ce5010b470fde220a80c1280439a3135` |
| `docs/migration/fixtures/pilot-fx/dissolve-representable-manifest.json` | `07be3cb5bdd6ac11bd904d7da2ae9d07b0563a7146f203494b3cc9ab28969cf0` |
| `crates/core/tests/pilot_dissolve_representable_import.rs` | `de3083ace6d38571fcb5f8672d1a57ce5194213218f2c224d92745b5e384ee08` |
| `docs/migration/p4b-representable-analytic-plan.md` | `be1bae7c5992c6208b57e83f72c18b55c0c6d1761765f073809447d4cc4df3ca` |
| `…/dissolve-representable-assets/flat-141c2c.png` | `6dc64f95194df9d40b9670d9604310bde48bf88160cfedf6fe14c5ed462b0106` |
| `…/flat-14302c.png` | `711181479b02f4151dd3ceb9755f8aa929eb394c702c29d7208519e88a93aca7` |
| `…/flat-182860.png` | `37038ee12831ba84e3612bb56614eaefdfb3380a87b3333fcd2d48c3a360f960` |
| `…/flat-20c8a0.png` | `c215e6f0bb55b5648c2f6a5915a7b5e2189768d1255cec48776bfd91d8b06ff1` |
| `…/flat-282c14.png` | `30817a217dabf93631ce3b5def02d29d41f60da59288f31832d4fed3b5beb400` |
| `…/flat-2c1c14.png` | `2cd68710c20c326a546a4f7e9b955e959f7a9e402234d0aa9750b8036afcac09` |
| `…/flat-404c61.png` | `73d751a272bc1691a14577427ebe1f2977abb5a78d8f2262a6e3ea16826a3683` |
| `…/flat-526078.png` | `efdcb7f2e31f38259a1a8397ee202945b047c313f7ce5f8972054fe6a6ed541b` |
| `…/flat-602818.png` | `31a288daf14a0158675bd5d06357ebd19cf0df301898025ba936022d4f316a3c` |
| `…/flat-634d41.png` | `d6188a2f14fe2d5f9e6e4a0ead58e4fd5bd2b2c4064e53826e01690967ef522e` |
| `…/flat-695344.png` | `91bff178251232f4aee6e753275bdce989d0bc9af9d1bee4b641cdc049b654a8` |
| `…/flat-e8b820.png` | `b0b4748bf4b3fd4a4f71e74673c232c4c5e154103e267c17a9b98a30308f1c58` |

(O manifesto do dataset registra o sha256 do manifesto S0 e das suas 33 fontes por referência; **nenhuma fonte nova da origem foi
lida**; o pin `d40ac3f7c0` está ausente neste clone e é declarado não verificado.)

## 7. Emendas ao plano (todas registradas, nenhum limiar mudou)

1. Antes de qualquer código: `cam-frames` do shot B de `x` e `ctl` passou de 40 para 42 (a câmera deve cobrir `cam-in 12 + len 30`).
2. Antes de qualquer código: seção "câmera cobre o shot, hold só como vizinho" (BL082.4.23), com as recusas `REP_CAMERA_SHORT` e
   `REP_PRIMARY_CLAMPED`.
3. Depois da **primeira** execução do gerador: a alegação `LIN` do controle `ctl` foi retirada (3 sondas em 1 quadro, peso baixo no
   centro: decisão 3 do coordenador). As alegações dos cenários a, c, d (peso alto do vizinho) foram todas atingidas de primeira.
4. Esclarecimento: `FLG` (flag `held`) não é um relógio do vizinho; age nos estados de todo deck gravado.

## 8. Roteiro do consumidor (S1b → S2), sem API nova aqui

1. **S1b:** carregar `bundles[id]` do JSON, importar com a política `Accept`, avaliar os dois decks (`evaluate_shot_at_offset` no vizinho
   fora do shot) e comparar `instance_frame`, `camera_clip_frame`, `held_instance/camera`, `scene_seconds` e as luzes com `frames[].decks`.
   Como a câmera cobre o shot, o plano primário e o offset **coincidem** nestes cenários (sem a divergência BL082.4.23).
2. **S2 T06/T10/X:** renderizar nos pixels de `frames[].probes[].pixel` e comparar com `expected` (tolerância 1/255 por canal e viés
   médio ≤ 0,25 com ≥ 24 sondas); os `deck_a`/`deck_b` dão o código esperado de cada deck antes da mistura; `rejects` diz que erro cada
   sonda separa. A alegação "pixel de A com hold" só vale depois disso, e não para os `lit-*-held` antigos.
3. Um consumidor que passe estes dados **não** prova ainda ripple, UI, export nem paridade com os 59 casos.

## 9. Patch literal para o coordenador (BL082.4.17 e F064)

Não aplicado aqui (comuns somente leitura). Linha para `docs/backlog.md`, depois de `BL082.4.15`:

```
| BL082.4.17 | P4b/S0-X representável | Cenários sintéticos VÁLIDOS com hold do vizinho fora do shot (instância e câmera cobrem o shot), mutantes só no vizinho e bundle de destino importável | A validar — oráculo + importabilidade na PR da branch `feat/p4b-representable-x`; consumidor S1b/S2 pendente | BL082.4.1, BL082.4.2 (S1a) | Cursor Sonnet5.5 / coordenador | [plano](migration/p4b-representable-analytic-plan.md) e [relatório](migration/p4b-representable-analytic-report.md). 7 cenários, 98 quadros, 37 testes Python (normal e -O), 5 testes Rust do core real; `generate --check` byte a byte; `ORIGIN_NOT_EXECUTED`/`ORIGIN_GOLDEN_NOT_COMPARABLE` valem, pin ausente não verificado. Não substitui os `lit-*-held` antigos (que seguem sem alegação de pixel de A) nem fecha BL082.4.23. Pixel/dois decks: S1b/S2. |
```

Parágrafo para `docs/features/fx-transitions.md` (F064), depois de "Referência analítica X de hold misto e relógio real da luz (P2-2)":

```
### Referência X representável: hold do vizinho fora do shot (BL082.4.17, A validar)

Os três cenários `lit-*-held` da S0 dão a A uma instância de 12 quadros sob um shot de 30: a origem prende dentro do shot, o importador
recusa (`SHOT_EXCEEDS_INSTANCE_DURATION`). A extensão `scripts/pilot_dissolve_representable.py`
(`docs/migration/fixtures/pilot-fx/dissolve-representable-{cases,manifest}.json`) traz 7 cenários **válidos**: instância e câmera cobrem o
shot e o hold existe só no deck vizinho fora do seu intervalo (cauda de A, cabeça de B), com instância e câmera presas por separado,
`held` falso no último quadro real e luz sempre em `f / fps` racional (inclui 30000/1001 no quadro 1800). Os mutantes de relógio valem só
para o vizinho; a matriz de hipóteses reporta as variantes que não discriminam. Cada cenário tem um bundle completo que o core real importa
sob a política interna `Accept` (teste Rust: importabilidade, limites, mapeamento dentro do shot, câmera, marcador e luz); a importação
pública continua `FX_TRANSITION_NOT_YET`. É `origin-formula` sintético, não golden executado nem aceito pela origem; a avaliação dos dois
decks e qualquer pixel são da S1b/S2. T06/T10/X só alegam pixel de A com hold nestes cenários depois desse consumidor.
```

## 10. Pendências e riscos

* Nenhum consumidor validou os dados (S1b em revisão na PR63, S2/S3a não iniciadas): **a validar**.
* BL082.4.23 (clamp do plano primário legado) segue aberto e fora desta entrega.
* O pin `d40ac3f7c0` continua ausente neste clone; `--require-pin` sai com 4 por desenho.
* O JSON tem ≈ 360 KB (14 quadros × dois decks × 7 cenários e os bundles): aceitável como fixture, mas é o maior arquivo novo.
* Commits: o harness do Cursor injeta um trailer `Co-authored-by` nos commits; os commits locais **ainda não publicados** são
  sanitizados antes do primeiro push (sem force), como a regra pede. A skill `commit` **não existe** neste ambiente (registrado).

## 10. Reforço do coordenador após a R1

R1 independente Windows aprovou93246d1 sem bloqueio:37specs normal/-O,14arquivos byte a byte,
verify normal/origin e strictpin4ausente,5Rust,fmt/clippy/tree. Frações independentes conferiram
7cenários/98quadros,390sondas,12PNGshashes. Mutantes adicionais do consumidor falso recusados.
Mac coordenador também reproduziu37normal/-O,14arquivos e5Rust, sem executar origem/runtime.

Nits corrigidos: removeram-se primary/neighbour do nível do cenário (sempre usar frames[]);
a guarda de imports usa lista explícita stdlib e parser3.9, sem atributo exclusivo de3.10;
Rust confere alinhamento/duração e transition_window contra a janela da referência; a convenção
de materialização mapeia os dois prefixos de assets pelo basename e hash. Spec confere o mapa.
Sondas, estados, bundles, hipóteses, limiares e12PNGs foram comparados com932 e estão idênticos.
Só os campos-resumo enganosos e a convenção do mapa mudaram nos cases; manifesto regenerado.

Limites da R1 preservados: câmera presa não tem discriminante em pixel quando pose satura;
UNH em cena estática precisa do estado. Viés agregado por cenário vs texto por quadro deve ser
decidido antes do consumidorS2. O plano inicial se refere à S1b737; o reforço posterior dessa
PR passou a comparar câmera/offset/pose, sem fechar o metadado legado(.23). Nada aqui afirma
pixels ou avaliação de dois decks. Os gates do reforço ainda precisam de execução/rechecagem.

Hashes após os ajustes de metadados/testes (os valores do §6 são históricos de932):

- scripts/pilot_dissolve_representable.py: c855b9e7206d584b6bed157b1b4916ff6dd7107a96b74b1b20f6177f1a03b575
- scripts/pilot_dissolve_representable.spec.py: 585ff2ec8081cd2c3aff4381f9f57830d352662d3db670c5293bb34d8e8d491f
- crates/core/tests/pilot_dissolve_representable_import.rs: 4dfdf1cdf4b3035b3a17b098e5b6d819a12fa17e6c428631d8612a789611aad0
- docs/migration/fixtures/pilot-fx/dissolve-representable-cases.json: 7de67565061aaa5ec5d87e1dfb60ea536eec52f4b8a89d6d9091e41500c6a276
- docs/migration/fixtures/pilot-fx/dissolve-representable-manifest.json: cb80d2ef88803e6b14b4825474639a523680c1d434415e07e79886dde536cebe
