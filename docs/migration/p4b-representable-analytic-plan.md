# P4b — X estendido com cenários sintéticos representáveis (detalhe do BL082.4.17)

**Status: plano pré-registrado, escrito ANTES de qualquer código do gerador, da spec ou do teste Rust.** Este arquivo é
detalhe de BL082.4.17 (backlog único em [docs/backlog.md](../backlog.md)); **não** é fila de execução e **não** cria IDs.
O relatório com os resultados é [p4b-representable-analytic-report.md](p4b-representable-analytic-report.md). Contrato
canônico: [p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md) §§6–8, 12–14 (D14/D15/D17, T06/T10/T16/T27/T29);
feature [F064](../features/fx-transitions.md). Base `main` `b6cf304` (PR60 integrada), ramo `feat/p4b-representable-x`.

## 1. Problema confirmado (revisão da PR63)

Três cenas `lit-*-held` do oráculo S0 (`lit-center-held`, `lit-end-held`, `lit-start-held`) dão ao shot A uma instância de
**12 quadros sob um shot de 30**. A origem tolera (prende dentro do shot); o importador do Tessel recusa
(`SHOT_EXCEEDS_INSTANCE_DURATION`, contrato da S1a). O consumidor S1b reconstrói a instância mais longa e **pula todo o deck
A** nos 30 quadros dessas cenas: a luz de A/B e os estados de B ficam cobertos, mas instância, câmera e `held` de A não.
**Não se corrige** aceitando o documento inválido nem regravando o JSON antigo. Consequência: S2 (T06/T10/X) não pode
alegar "pixel de A com hold" para esses três casos. Este trabalho entrega **variantes válidas**, não uma falsa equivalência.

Fora do escopo desta entrega: o motor, efeitos, JS, o importador (não se enfraquece), a API da S1b (`evaluate_shot_at_offset`,
`ScenePlan.transition`), runtime, UI e a abertura do portão público. O corpus S0 (`dissolve-cases.json`,
`dissolve-manifest.json`, `pilot-dissolve-corpus.py`, `pilot_dissolve_analytic.py` e suas specs) **não é editado**; é lido e
importado como biblioteca.

## 2. Decisões do coordenador, como entram aqui

1. Todo shot tem `instanceDuration ≥ instanceStart + len` e `cameraIn ≥ 0`, entradas finitas e inteiros estritos.
   O hold de A ocorre **na cauda, como vizinho, fora do seu intervalo**; nunca encurta a instância dentro de um shot.
   Instância e câmera prendem **separadas**: `held = false` no último quadro real e `true` só fora dele. A luz anda
   **sempre** em `f / fps` racional, mesmo com os dois clocks presos.
2. Quatro cenários obrigatórios (a–d) e três extras (seção 4).
3. Cada cenário só alega prova do relógio da luz **no deck vizinho**: o erro do mutante é aplicado **apenas ao vizinho**; o
   deck primário usa a verdade nos dois lados da comparação. Nenhum erro do primário sustenta uma alegação do vizinho.
4. Limiares iguais aos da S0, registrados **agora** (seção 5), nunca relaxados depois de ver o runtime.
5. Provas **por quadro, dos dois decks, completas** (seção 6); sem pular um deck; sem adaptar a duração do candidato
   depois de comparar.
6. Um **bundle de destino completo por cenário**, importado pelo core real com `import_value_admitting(.., Accept)` (S1a),
   sem alterar produção; o portão público continua `Refuse`.
7. Gerador stdlib usa os kernels Python da S0 (independentes do Rust); a spec tem **vetores à mão** e **recomputo
   independente** (`Decimal`/`Fraction`/`ctypes`), além de mutantes fortes e controles.
8. Dataset `origin-formula/synthetic`, **destination-validity-checked**; não é golden executado nem aceito pela origem.
9. `generate --check` byte a byte, `verify` honesto, normal e `-O`, com e sem a origem nos limites reais.

## 3. O que o desenho muda em relação à S0

| S0 (`pilot_dissolve_analytic.py`) | Esta extensão (`pilot_dissolve_representable.py`) |
|---|---|
| Hipóteses de relógio aplicadas aos **dois** decks (um erro de luz do primário podia carregar a sonda) | Hipótese de relógio aplicada **só ao vizinho**; o primário é a verdade dos dois lados |
| Três cenas `lit-*-held` com instância 12 sob shot 30 | Instâncias e câmeras cobrem o shot; o hold aparece só na cauda/cabeça vizinha |
| Sondas contadas por entrada (dois marcadores no mesmo pixel contavam duas vezes) | Sondas contadas por **par (quadro, pixel) distinto** |
| Luz com amplitude baixa (cores base 28–52): variação do flicker ≈ 1–2 códigos | Cores base escolhidas para o pixel iluminado ficar ≈ 0,9 linear no nível 1: variação ≥ 4 códigos |
| Sem bundle de destino | Bundle completo por cenário, importado pelo core |
| Quadros: `ks` escolhidos | **Todos os 12 quadros da janela** + o quadro antes e o depois, estados dos dois decks |

Os kernels (flicker, câmera, projeção, evento, luz, mistura) são os da S0, transcritos do texto da origem (ponta
`3655790`); a **única** coisa nova no cálculo é a regra de aplicação (vizinho apenas) e dois mutantes novos (`UNH`, `FLG`).
Nenhuma fonte nova da origem foi lida; as 33 fontes ficam referenciadas pelo sha256 do manifesto S0.

## 4. Cenários pré-registrados (números literais)

Constantes comuns: transição `crossDissolve`, `durationFrames 12`, `easing easeInOutCubic`, `curve smooth`, `intensity 1`,
`handles hold` (exceto o controle), viewport 640×360, FOV vertical 50°, saída opaca. Shots `A` (`shot-a`) e `B`
(`shot-b`) com corte `C = B.start`. Cada cenário registra **todos** os quadros `[janela.início − 1, janela.fim]` (14).

| ID | Kit | fps | Alinh. | `C` | edição | A (start, inst-start, inst-frames, cam-in, cam-frames) | B (idem) |
|---|---|---|---|---|---|---|---|
| `rep-a-tail-both-held-light` | lit | 24/1 | start `[284,296)` | 284 | 344 | 0, 210, 494, 119, 403 | 284, 0, 60, 0, 60 |
| `rep-b-tail-instance-held-camera-runs` | pos | 24/1 | start `[30,42)` | 30 | 60 | 0, 4, 34, 0, 48 | 30, 0, 60, 0, 40 |
| `rep-b2-tail-instance-edge-camera-runs` | pos | 24/1 | start `[30,42)` | 30 | 60 | 0, 4, 39, 0, 48 | 30, 0, 60, 0, 40 |
| `rep-c-head-both-held-light` | lit | 24/1 | end `[463,475)` | 475 | 535 | 0, 0, 475, 0, 475 | 475, 0, 60, 0, 60 |
| `rep-d-ntsc-tail-held-light` | lit | 30000/1001 | start `[1800,1812)` | 1800 | 3600 | 0, 0, 1800, 0, 1800 | 1800, 0, 1800, 0, 1800 |
| `rep-x-center-both-mixed` (extra) | both | 24/1 | center `[24,36)` | 30 | 60 | 0, 4, 60, 0, 30 | 30, 0, 60, 12, 40 |
| `rep-ctl-real-both` (controle, `handles use`) | both | 24/1 | center `[24,36)` | 30 | 60 | 0, 4, 60, 0, 40 | 30, 10, 60, 12, 40 |

Cada shot tem comprimento `len = end − start` e `inst-frames ≥ inst-start + len`; `cam-frames` pode ser menor que
`cam-in + len` somente quando o importador aceita a câmera curta (**nenhum** cenário depende disso: todas as câmeras acima
cobrem o shot).

Estados esperados que cada linha **obriga** (todos conferidos pelo gerador, quadro a quadro):

* **a**: A em cauda, `delta = f` = 284..295, instância querida 494..505 → presa em **493** (`held_instance = true` em todo
  o k), câmera querida 403..414 → presa em **402** (`held_camera = true`). `f = 283` (último quadro de A): instância 493 e
  câmera 402, **`held = false`** nas duas. A tem peso quase 1 em `k = 0..4`.
* **b**: A em cauda; instância querida 34..45 → presa em **33** (`held_instance = true`); câmera querida 30..41, o clipe tem
  48 quadros: **real** (`held_camera = false`), a câmera segue andando além do shot. O marcador de A (`x = 10,8`, evento
  `start 27`) fica parado em `p = 0,5` (y = 0) enquanto a câmera o desliza pela tela; o grampo conjunto prenderia a câmera
  em 29.
* **b2**: igual a **b**, mas a instância de A tem **5 quadros reais de cauda**: instância querida 34..45, última real 38.
  `f = 34` (`k = 4`): instância 38, **`held = false`** (último quadro real); `f = 35` (`k = 5`): querida 39 → presa em 38,
  **`held = true`**. Evento `start 31`.
* **c**: B em cabeça (janela `end`, primário A): `delta = f − 475` = −12..−1, instância e câmera queridas negativas →
  presas em **0** (`held` os dois). `f = 475` (primeiro quadro real de B, já fora da janela): instância 0 e câmera 0,
  **`held = false`**. B tem peso alto no fim (`k = 6..11`).
* **d**: 30000/1001, `C = 1800` (≈ 60,06 s): A em cauda presa em **1799** (instância e câmera); a luz do vizinho anda em
  `f · 1001 / 30000`, não em `f / 30`.
* **x** (extra): hold misto nos dois lados do corte (B cabeça: instância presa em 0 com câmera real `12 + delta`; A cauda:
  instância real com câmera presa em 29), luz e marcador. Peso baixo nas pontas: **não alega isolamento**.
* **ctl** (controle): nada preso (`handles use`): o grampo conjunto coincide com a verdade; o relógio da luz da instância
  difere de `f / fps`.

### Kits (dados, sem assets privados)

* `lit` (cores novas, escolhidas **antes** de qualquer execução para que o pixel iluminado fique ≈ 0,9 linear no nível 1 do
  flicker, de modo que nem o separador nem o limite 1 sejam apagados): A `backdrop (64,76,97)`, `marker (82,96,120)`;
  B `backdrop (99,77,65)`, `marker (105,83,68)`. Plano de fundo 60×12 em z = −2, marcador estático 1×1 em `(1,5; −0,5; 0)`,
  câmera `hold` em `(0,0,6) → (0,0,0)`, uma lâmpada por deck: `lamp-a` em `(−1,5; 1; 2)` cor `(1; 0,7; 0,4)` e `lamp-b` em
  `(1; 1,5; 2)` cor `(0,4; 0,7; 1)`, intensidade 20, raio 20, flicker 1 (as mesmas lâmpadas da S0).
* `pos` e `both`: as cores e a lâmpada da S0 (`KIT_COLOURS`, `LAMPS`), marcador 0,6×0,6 com evento
  `localPosition += (0; 4,8; 0)` em 12 quadros de instância a partir de `start`, base y = −2,4; câmera `truckRight` a
  **0,36 unidade por quadro** (`distance = 0,36 · (frames − 1)`, 9 casas), `from initial`, `linear`. Marcadores: **A**
  `x = 10,8` (cenários b/b2), `x = 7,5, start 30` (x e ctl, como na S0); **B** `x = 0` (b/b2) e `x = 2,2, start 0` (x e ctl).

### Por que `C = 284`, `475` e 1800 e por que `I = 210`, `Cin = 119`

Escolhidos por uma **busca anterior ao gerador**, só sobre o kernel do flicker (sem pixel, sem produto, sem ver limiar
nenhum): maximiza o terceiro maior desvio relativo ponderado pelo peso do vizinho entre o nível da verdade e o nível do
relógio errado, em `k = 0..5` (a) ou `k = 6..11` (c). Resultado: a: 0,42; c: 0,196. Para d (corte fixo em 1800, o
caso NTSC grande) não houve busca: o desvio entre `f · 1001/30000` e `f/30` é o que o fps dá. A busca **não** relaxou limiar; ela escolheu números de desenho. Sem ela, os níveis de `lamp-a`
(0,71–0,74) em 30..35 não deixariam sequer 4 códigos de diferença.

## 5. Hipóteses (mutantes) e limiares pré-registrados

Todas as hipóteses de **relógio** valem **só para o deck vizinho** (`scope: neighbour`); `DSW` e `NOH` valem na mistura.

| ID | Regra errada |
|---|---|
| `PIN` | grampo conjunto: um único deslocamento limitado ao intervalo que instância e câmera têm em comum |
| `SWP` | instância calculada com a câmera e câmera com a instância (início/comprimento trocados) |
| `PRI` | vizinho usa início, instância e câmera do shot primário |
| `UNH` | a instância **não** é presa no fim (corre além do último quadro); a câmera segue a verdade |
| `FLG` | `held` marcado no último quadro real (`querido ≥ último`) e no primeiro (`querido ≤ 0`) |
| `LIN` | luz em `instância / fps` (vizinho preso congela o flicker) |
| `LSR` | luz em `(f − início do shot) / fps` |
| `LCC` | luz em `câmera / fps` |
| `LCE` | luz em `min(f, fim−1) / fps` (ou `max(f, início)` na cabeça) |
| `LNF` | luz em `f / round(num/den)` (30 em vez de 30000/1001) |
| `DSW` | `mix(b, a, …)`: decks trocados (equivale a pesos espelhados, `1 − w`) |
| `NOH` | progresso sem o meio quadro: `ease(k / D)` em vez de `ease((k + 0,5) / D)` |

`FLG` age sobre os estados (`held`), não sobre pixel: um cenário só a rejeita se algum quadro tiver um último/primeiro
quadro real. Os demais agem sobre pixel.

**Limiares (iguais aos da S0; não são retocados depois):** separação ≥ **4 códigos** em algum canal; ≥ **3 sondas** em ≥ **2
quadros**; controles que devem concordar: **no máximo 1 código** em toda sonda; viés médio com sinal |média(Δ)| ≤ **0,25**
(com ≥ 24 sondas por canal) e Δ máximo ≤ 1 como **tolerância do consumidor** (importada de `dissolve-cases.json`, não derivada
de resultado). Cores de deck quantizadas a 8 bits (meio para cima) **antes** da mistura. `centre_px`/`rect_px` são
**diagnósticos**, sem portão de posição. Centro do pixel `i + 0,5`, FOV 50 vertical, 640×360, margem plana de **6 px**, alfa de
saída 255. Luz: intensidade `[0, 20]`, cor e flicker `[0, 1]`, raio `> 0`, no máximo **4** lâmpadas. Câmera, evento, flicker e
mistura como em `three-scene-runtime.ts`/S0. Uma sonda é **(quadro, pixel)** distinta. Se o desenho não excluir um mutante que
o cenário alega, o gerador **recusa** a geração (`REP_NO_REFUSAL_POWER`) e o desenho é explicado e refeito; o limiar não
muda. Hipótese que **deve** concordar e discorda: `REP_CONTROL_DISAGREES`.

**Alegações por cenário (pré-registradas).** `rejects` = deve atingir os limiares; `agrees` = ≤ 1 código em toda sonda.

| Cenário | `rejects` (só vizinho, salvo DSW/NOH) | `agrees` |
|---|---|---|
| a | `LIN`, `LCC`, `LCE`, `DSW`, `NOH`, `FLG` | `LSR` (A começa em 0: relógio do shot = relógio da edição), `LNF` (24/1), `PIN`, `SWP`, `PRI`, `UNH` (cena estática) |
| b | `PIN`, `SWP`, `PRI`, `UNH`, `DSW`, `NOH`, `FLG` | `LIN`, `LSR`, `LCC`, `LCE`, `LNF` (sem luz) |
| b2 | `PIN`, `SWP`, `PRI`, `UNH`, `DSW`, `NOH`, `FLG` | `LIN`, `LSR`, `LCC`, `LCE`, `LNF` (sem luz) |
| c | `LIN`, `LCC`, `LCE`, `LSR`, `DSW`, `NOH`, `FLG` | `LNF` (24/1), `PIN`, `SWP`, `PRI`, `UNH` (cena estática) |
| d | `LNF`, `LIN`, `LCC`, `LCE`, `DSW`, `NOH`, `FLG` | `LSR` (A começa em 0), `PIN`, `SWP`, `PRI`, `UNH` |
| x | `PIN`, `SWP`, `PRI`, `DSW`, `NOH` | (nenhum; luz e `UNH` só são **reportados**: peso baixo no centro) |
| ctl | `SWP`, `PRI`, `LIN`, `DSW`, `NOH` | `PIN`, `UNH` (nada preso) |

Toda hipótese, em todo cenário, entra na **matriz de hipóteses** do JSON com o estado medido (`rejects`, `agrees` ou
`partial`, mais contagem de sondas e de quadros): as **variantes não discriminantes são reportadas**, não escondidas.

## 6. Estados esperados por quadro (os dois decks, sempre)

Para cada quadro `f` (14 por cenário) e cada deck (`A`, `B`): `edit_frame`, `edit_seconds` (**fração reduzida** `f · den /
num` e decimal de 9 casas), `delta = f − shot.start`, `instance_wanted`, `instance_frame`, `held_instance`, `camera_wanted`,
`camera_clip_frame`, `held_camera`, `shot_clip_frame` (regra de borda `shot_clip_frame` do corpus S0, para um clipe
hipotético que cobre o shot inteiro) e `fx_edge` (verdadeiro quando `f` está fora do intervalo do shot do deck), `scene_seconds` (relógio da luz = `edit_seconds`), as lâmpadas
(`flicker_level`, `effective_intensity`), a pose da câmera, a posição do marcador (kits móveis). Nos quadros da janela:
`k`, `progress`, `shaped`, `amount`, `weight` e as sondas, com o código de cada deck e o `expected` final **nos pixels
prescritos**. `held_*` é verdadeiro **somente** quando o quadro querido está fora de `[0, frames − 1]`. O gerador recusa
(`REP_HELD_FLAG`) um estado cujo `held` não bate com `querido ≠ preso`.

## 7. Bundle de destino e o que o Rust prova

Um bundle completo (`schemaVersion 1.0`, objetos, sem conversor) por cenário, em `bundles` do JSON de casos: assets PNG
planos de **1×1 pixel**, gerados deterministicamente (blocos *stored*, sem dependência do `zlib`) em
`docs/migration/fixtures/pilot-fx/dissolve-representable-assets/`, `contentHash` = sha256 real do arquivo; cenas `scene-a`/`scene-b`
(ids de nó **colidentes** `root`/`backdrop`/`marker`), `lighting` (ambient 1, 1 lâmpada), trilha de performance com o canal
`event` (`holdFinal true`), clipes de câmera `hold` ou `truckRight`, e `fx.transitions` com a transição do cenário.

`crates/core/tests/pilot_dissolve_representable_import.rs` (só `std`/`serde_json` existentes; **não** executa Python) lê o
JSON versionado e prova, no core **real** de `main`:

1. cada bundle **importa** com `import_value_admitting(.., Accept)` e a importação pública (`import_value`) o **recusa** com
   `FX_TRANSITION_NOT_YET` (portão fechado);
2. **controle negativo**: a pré-condição antiga (instância de 12 sob shot de 30) continua **recusada**
   (`SHOT_EXCEEDS_INSTANCE_DURATION`);
3. limites de instância (`inst-start + len ≤ inst-frames`), câmera (`cam-in` inteiro ≥ 0; comprimento do clipe do core =
   o do JSON) e luz (`Lighting::read` aceita; ≤ 4 lâmpadas, intensidade `[0,20]`, cor/flicker `[0,1]`, raio > 0);
4. o que o core de `main` já avalia fora de qualquer API nova: mapeamento instância/câmera **dentro** do shot, tempo de
   edição racional exato, intensidade efetiva da luz em `f / fps` real (inclusive 30000/1001), pose da câmera e posição do
   marcador em cada quadro de instância/câmera presos;
5. coerência interna dos estados (`held` só fora do intervalo; presos = grampo recomputado dos números **importados**).

**Não prova:** avaliação de dois decks, `held` fora do shot pelo core (S1b), `ScenePlan.transition`, qualquer pixel
(S2). O consumidor futuro, que depende da S1b, está descrito no relatório.

## 8. Reprodução, gates e ownership

```
python scripts/pilot_dissolve_representable.py generate --check          # byte a byte, offline
python scripts/pilot_dissolve_representable.py generate --origin <repo>  # regrava, relê a origem
python scripts/pilot_dissolve_representable.py verify [--origin <repo>] [--require-pin]
python scripts/pilot_dissolve_representable.spec.py                      # e `python -O`
./scripts/cargo test -p tessel-editor-core --locked --test pilot_dissolve_representable_import
```

Interpretador real **3.13.17** (`%LOCALAPPDATA%\Programs\Python\Python313\python.exe`); o atalho da Microsoft Store é
recusado, sem `SKIP`. Gates: Python (spec normal e `-O`, `generate --check`, `verify` com e sem a origem), Rust (o novo
teste sobre `main` `b6cf304`, `cargo tree -p tessel-editor-core` sem `gpui`/`wry`), `cargo fmt --all -- --check` e
`clippy -p tessel-editor-core --all-targets --locked -D warnings`. Sem GPU, sem lock de desempenho, sem build do app, do
node, do motor ou do workspace inteiro. Origem (`E:\www\tesselapp`, suja, 151 arquivos): **somente** `git cat-file`/
`rev-parse` dos blobs fixados; nada modificado, instalado ou executado; o pin `d40ac3f7c0` pode estar ausente e **não** é
anunciado como verificado quando está.

**Arquivos novos (único ownership):** `scripts/pilot_dissolve_representable.py` e `.spec.py`;
`docs/migration/fixtures/pilot-fx/dissolve-representable-{cases,manifest}.json` e a pasta `dissolve-representable-assets/`;
`crates/core/tests/pilot_dissolve_representable_import.rs`; este plano e `p4b-representable-analytic-report.md`.
**Não editados:** corpus/manifest/scripts S0, fixtures legados, goldens, `crates/core/src`, runtime, ScenePlan/S1b, UI,
`Cargo.lock`/dependências, backlog, `docs/features/README.md`, `docs/README.md`, contratos concorrentes. Nunca
`TESSEL_UPDATE_FX_CONTRACT`. O patch literal de BL082.4.17/F064 fica **no relatório**, para o coordenador.

## 9. Vetores à mão (calculados fora do gerador, `Decimal`/`Fraction`/`ctypes`)

| Quantidade | Valor |
|---|---|
| `seedOf("lamp-a")`, `seedOf("lamp-b")` | 807477363, 807477364 |
| nível de `lamp-a` em `284/24` s (a, `k = 0`, verdade) | 0,811566769 |
| nível de `lamp-a` em `493/24` s (a, `LIN`), `402/24` s (a, `LCC`), `283/24` s (a, `LCE`) | 0,970547388; 0,962061105; 0,895748195 |
| nível de `lamp-b` em `463/24` s (c, `k = 0`), `0` s (c, `LIN`/`LCC`), `475/24` s (c, `LCE`) | 0,832962015; 0,739396138; 0,752307966 |
| nível de `lamp-a` em `1802 · 1001/30000` s (d, verdade), em `1802/30` s (`LNF`), em `1799 · 1001/30000` s (`LIN`) | 0,725952789; 0,622278394; 0,590289496 |
| pesos `weight(k)`, `k = 0..11`, easeInOutCubic + smooth, D = 12 | 0; 0,000182; 0,003830; 0,027595; 0,114713; 0,330723; 0,669277; 0,885287; 0,972405; 0,996170; 0,999818; 1 |
| `tan(25°)` | 0,466307658154998592830006194799559451311… |
| centro em pixel do marcador estático `(1,5; −0,5; 0)`, câmera em `(0,0,6)`, 640×360 | `(416,502811423; 212,167603808)` |
| deck A (`lit`), pixel `(160, 90)` no fundo (z = −2), `k = 0` de a: ponto de impacto | `(−3,305603177; 1,854868240; −2)` |
| mesmo pixel: `d`, `n·l`, `falloff² · n·l` | 4,471129895; 0,894628448; 0,811489143 |
| mesmo pixel: código não arredondado `(R, G, B)` com as bases `(64, 76, 97)` | `(221,475069; 223,095556; 224,523004)` → `(221, 223, 225)` |
| janela `start`, C = 284, D = 12 | `[284, 296)`; `end`, C = 475: `[463, 475)`; `center`, C = 30: `[24, 36)` |

Os vetores acima são o **mínimo** que a spec recomputa com meios independentes do gerador e compara com o JSON gerado.
