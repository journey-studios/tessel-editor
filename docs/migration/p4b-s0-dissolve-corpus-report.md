# P4b-S0 — corpus e oráculo do cross dissolve (relatório)

Frente **BL082.4.1**, feature **F064** ([fx-transitions.md](../features/fx-transitions.md)). Plano:
[p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md) (v2). Esta entrega é só o **oráculo executável e o corpus**;
não há Rust, runtime, JS de produção, GPUI nem nó de árvore. A feature **não está implementada**.

## 1. O que foi entregue

| Arquivo | Papel |
| --- | --- |
| `scripts/pilot-dissolve-corpus.py` | Oráculo de referência (Python stdlib), gerador/verificador do corpus e do manifesto |
| `scripts/pilot-dissolve-corpus.spec.py` | 161 testes na versão original (167 hoje, §10): vetores à mão, aritmética exata, mutantes, adulteração, CLI, verificador de origem |
| `scripts/pilot_dissolve_analytic.py` (§10, P2-2) | Referência analítica X de hold misto e do relógio real da luz: cenas sintéticas, cinemática, projeção, luz e mistura por fórmula; só stdlib; não importa o script do corpus |
| `scripts/pilot_dissolve_analytic.spec.py` (§10) | 36 testes: vetores à mão, avaliador independente que refaz todas as sondas, portão de consumidor, mutantes do próprio oráculo, estrutura |
| `docs/migration/fixtures/pilot-fx/dissolve-cases.json` | Corpus canônico (seções: janela, inteiros estritos, progresso, quantização, pixels, quadros de deck, handles, `explain` ×31, ids, alpha, tracer bullet, limites, `resource_union`, tolerâncias) |
| `docs/migration/fixtures/pilot-fx/dissolve-manifest.json` | 26 fontes com digest, origem, autoria (head do plano), bloqueios e divergências documentadas |
| `docs/features/fx-transitions.md` | Contrato F064 (lib/JSONL/UI **Pending**) |
| `docs/features/effects.md` | Só a nota de rascunho superado e a divergência de alpha; a citação de F016 (tolerância 16, maior mudança 69) está intacta |

O corpus e o manifesto são JSON canônico (chaves ordenadas, LF, newline final), sem caminho privado. A única data é a
literal `2026-11-14` (reconferência de F016).

## 2. Cabeçalhos de fonte e origem (registrados antes de publicar)

| Item | Valor |
| --- | --- |
| Head do plano lido | `2395719046875a06c4986e4e8a949a448bfcb9d3` (PR [#58](https://github.com/journey-studios/tessel-editor/pull/58), integrado por fast-forward, sem force) |
| Branch | `feat/p4b-s0-dissolve-corpus` |
| Ponta do clone da origem (`tesselapp`, só leitura via `git`) | `365579076782c1fc19c6464f3bc0199b30c0ab8f` |
| Pin da delegação | `d40ac3f7c0` — **não é commit neste clone** (`PIN_COMMIT_ABSENT`) |
| Fontes | Original: 26 (21 do atestado do coordenador, 19 idênticas à ponta e 2 diferem, + 5 só da ponta). **Hoje (§10): 33 = 21 atestadas + 12 só da ponta** (as 7 novas servem à referência X) |
| `PIN_BLOB_UNVERIFIED` | `fx/fx-schema.ts` (`5ce474989c`) e `schema/render-bundle.schema.ts` (`5bdbb49749`): objeto ausente neste clone, **ficam no atestado do coordenador**; a diferença é só comentário em `fx-schema` e narração em `render-bundle.schema` |
| Origem executada? | **Não.** `origin-blob verified ≠ origin executed`: `ORIGIN_NOT_EXECUTED` e `ORIGIN_GOLDEN_NOT_COMPARABLE` ficam no manifesto. Nenhum golden foi lido, comparado ou mutado; nenhum motor JS foi rodado |

`verify --origin … --require-pin` sai com código **4** (`PIN_UNVERIFIED`) por desenho: o pin é inverificável aqui.

## 3. Python real no Windows (BL167.5, setup parcial)

- `python` no PATH é o **stub da Microsoft Store**: `…\WindowsApps\python.exe`, tamanho 0, `Archive, ReparsePoint`,
  `--version` sai 9009 ("Python was not found…").
- Python real instalado via winget: **3.13.17** (`tags/v3.13.17:64e080d`, MSC v.1944, AMD64), prefixo
  `%LOCALAPPDATA%\Programs\Python\Python313` (usuário omitido), `python.exe` de 105 696 bytes, sha256
  `7c2fc2b303a4ee3cc14cc17b47bc6e8bca98c78792edd123b846a560da44b203`.
- O oráculo e a spec **recusam** o interpretador impróprio (saída **5**, `INTERPRETER_NOT_REAL`): executável vazio, Python
  < 3.9, caminho em `WindowsApps` sem `PythonSoftwareFoundation`, arquivo de tamanho 0. Ambos imprimem `sys.version`.
- Isto cobre só o **Python** de BL167.5. Build de release e provas ao vivo seguem pendentes.

## 4. Portões e limites exatos (pré-registrados; nenhum pixel foi validado)

| Portão | Valor | Fonte/observação |
| --- | --- | --- |
| Monitor/render | máx. **1/255** por canal + viés \|média Δ\| ≤ **0,25** | Decks 8 bits; o viés é o que discrimina truncamento |
| H.264 | tolerância **16** | Importada de F016 (`CODEC_TOLERANCE`), com a justificativa de croma/quantização; a spec exige que ela discrimine |
| Teto do plano | 4 194 304 bytes, 20 000 itens, profundidade 1 | **A validar, não medidos** |
| Intensidade 0/0,5/1 (smooth) | 162 / 165 / 168 | Vetores do plano reproduzidos; **não** discriminam truncamento sozinhos (ver §6) |

Fórmula reproduzida: `t=(f−start+0,5)/D`, `progress=ease(t)`, `shaped=mix(progress, smoothstep, smooth)`,
`weight=mix(progress, shaped, clamp01(intensity))`, mistura em luz linear (IEC 61966-2-1), arredondamento
half-up `floor(x·255+0,5)` com guarda de empate 1e-6. Intensidade 0 **não** é identidade.

## 5. Divergências e decisões abertas (no manifesto)

| ID | Resumo |
| --- | --- |
| `ALPHA_VERDICT` | A origem recusa `alphaMode ≠ opaque` com fx ou dissolve legado (`UNSUPPORTED_ALPHA`); o destino aceita e desenha opaco. **Divergência de veredito**, nunca "aceito pela origem" |
| `DECKS_8_BIT` | Mistura só definida para decks de 8 bits |
| `KEYS_REFUSED` | Keyframes de transição recusados (`FX_TRANSITION_KEYS_NOT_YET`) |
| `NARRATION_OUT_OF_SCOPE` | Diferença de narração em `render-bundle.schema` fora do escopo; é decisão registrada |
| `LEGACY_FIXTURES_UNTOUCHED` | Corpus legado de P0/P2/P3/S10 intocado (nova faixa S6) |
| `HIDDEN_FALSE` | decidido: hidden true ou ausente nas transições; false recusado estruturalmente na S1a |
| `STRICT_INTEGERS` | Inteiros estritos (sem bool/float/str; 2^53−1; i64) |
| `DURATION_CHECKED_FIRST` | `D ≤ edit` verificado antes da aritmética; janela inválida é recusada, nunca clampada |

Proposta (não decidida): ordem de `used_by` em `resource_union` por `assetId` com qualificador de deck; os ids de nó
colidem entre decks.

## 6. Resultados das execuções (Windows x64, Python 3.13.17)

> **Histórico da versão original** (161 testes, 26 fontes, hashes abaixo). Os números de hoje estão em §10; nada daqui foi reescrito.

| Comando | Saída |
| --- | --- |
| `generate --check` | exit 0: reproduz os dois arquivos byte a byte, nada escrito |
| `generate --check --origin E:/www/tesselapp` | exit 0, com os 3 avisos `PIN_*` |
| `verify` | exit 0: 26 fontes, offline |
| `verify --origin E:/www/tesselapp` | exit 0: "origin-blob verified; the origin was NOT executed", 26 fontes |
| `verify --origin … --require-pin` | exit **4** `PIN_UNVERIFIED` (esperado) |
| spec (com `TESSEL_ORIGIN_PATH`) | 161 testes, **OK** (os 2 `RealOriginTests` rodam) |
| spec `-O` (com `TESSEL_ORIGIN_PATH`) | 161 testes, **OK** (sem `assert` nu) |
| spec sem origem | 161 testes, **OK (skipped=2)** |

Hashes (inalterados por `--check`/`verify`):

```text
c803136bc55a54538900e52bcc768fcd14d1c29c046b4e53e28ab037674006a4  dissolve-cases.json     197772 bytes
1bb516e032f5ec24eae06c2afadc8e254c87411bedd71c86f1b74cc30283711d  dissolve-manifest.json   22307 bytes
```

Fixtures legados intocados (`git diff HEAD` vazio em `bundles/`, `expected/`, `cases.json`, `manifest.json`):

```text
b9eae80ff9a4da3272fc128fe4be214491db2d3d71b48dc60339b19e08c6b25f  invalid-dissolve-transition.expected.json
8ab1dcd8210a41f5e77eca06116a8e1daa334ef8af84096b9dd127c78e7c40c5  invalid-dissolve-transition.bundle.json
43fd1a5568a63745c1043eab45cae028fa03a772eea11c9d0bac43e808718e2b  valid-cut-instance-offsets.bundle.json
```

Evidência de força do oráculo:

- **Mais de 30 mutantes** (`MutationTests`: janela clampada, easing ignorado, truncamento em vez de half-up, `intensity`
  ignorada, bool aceito como inteiro etc.) são mortos pela spec.
- **Truncamento:** em 216 sondas exatas (Decimal) o máximo de erro é 1 código, mas a média é −0,444 contra 0,0 do
  arredondamento correto. Por isso o viés, e não o máximo de 1/255, é o portão que discrimina. Os vetores 162/165/168
  sozinhos **não** matam o truncamento; `HAND_WEIGHTS` e as sondas exatas cobrem isso.
- **Aritmética exata:** 2352 sondas com 0 recusas, comparadas com Fraction/Decimal independentes do caminho float.
- **Adulteração/CLI:** `TamperAndCliTests` (20) cobre corpus ou manifesto alterados e os códigos de saída.

## 7. Limites desta entrega (não afirmar além)

- **Não é a feature:** nenhuma transição existe em lib, CLI, JSONL ou UI; o portão público `FX_TRANSITION_NOT_YET`
  permanece até a S2A.
- **Mac/cross-host não executado:** o `--check` em outro host (diferença de `pow` da libm) **não foi rodado**; a
  reprodutibilidade byte a byte está provada só em Windows x64.
- **Pin inverificável** neste clone; 2 blobs ficam no atestado do coordenador.
- **Origem não executada:** só blobs lidos via `git`; nada comparado com goldens.
- Sem GPU real, sem desempenho, sem áudio ouvido, sem ponteiro físico. O lock de GPU não foi tomado (só vale para
  desempenho).
- Tolerâncias e tetos são **pré-registrados / A validar**; nenhum pixel nem medida os confirma.
- Não é o framework de mutantes da S5.
- `pnpm test`/`pnpm tsc`/`pnpm build` **não se aplicam**: repositório Rust/Python e nenhum JS tocado. Nenhum Cargo foi
  executado.

## 8. Linhas para o coordenador (não editei `backlog.md`, `features/README.md` nem `docs/README.md`)

**BL082.4.1** (S0): status → **A validar**, até as correções de pin e de cobertura analítica da revisão fecharem; evidência: este relatório, o
commit/PR da branch `feat/p4b-s0-dissolve-corpus`, `scripts/pilot-dissolve-corpus.py` e `.spec.py` (161 testes),
`dissolve-cases.json`/`dissolve-manifest.json`. Pendências herdadas: Mac cross-host `--check`; pin inverificável.

**F064** (linha da matriz): `fx-transitions` — lib **Pending**, JSONL **Pending**, UI **Pending**; oráculo e corpus
validados (Windows); documento [fx-transitions.md](../features/fx-transitions.md).

**BL167.5** (parcial): setup do Python real concluído (3.13.17, stub da Store recusado pelo oráculo, §3); build de
release e provas ao vivo seguem **pendentes**.

## 9. Revisão independente e atualização de pin

O resultado original em `173f5f4` não foi aprovado como S0 completa. A revisão confirmou a
matemática por Decimal, mas encontrou `--require-pin` aceitando verificação offline ou só blobs
sem commit. O coordenador fechou esses caminhos com exit 4 e testes de Git real, sem mockar
`check_origin` (fixture com blob comum à ponta e blob antigo presente por id). Os nomes de tiers
agora distinguem pin-commit verificado de mera atestação.

Também corrigiu uso de `digests`, alias legítimo da Store e valor integral `2.0` na mensagem, além
de declarar a força real da checagem textual. O corpus ganhou esse vetor e a decisão hidden,
sem mudar fórmulas, tolerâncias nem fixtures legados. Contagens de 161/hashes de §6 são históricos
da versão original; as novas execuções serão identificadas separadamente.

**Pendente nesta atualização:** sondas analíticas X para hold misto e relógio da luz. O X atual
cobre estados temporais e cores constantes, não pixels de decks texturizados nesses casos.
A linha F064 existe na matriz com todas as superfícies de produto Pending; BL082.4.1 fica
A validar e BL167.5 só tem setup de Python, sem release/janela.

Execução da correção pelo coordenador no macOS, Python 3.14.3: spec com origem **166/166**
nos modos normal e `-O`; `generate --check` byte a byte; verificação estrita com origem **exit 0**
(commit do pin presente: 21 fontes atestadas e 5 só da ponta). Specs de tabelas do backlog
**19/19**, higiene **13/13**, `git diff --check` limpo. Não houve Cargo, engine ou GPU nesta rodada.
A atualização incorpora o plano final da PR58 (`002fe9f`, merge `1796107`); o relato de §2
continua identificando a versão anterior lida na implementação original.

## 10. Correção P2-2: referência analítica X para hold misto e relógio real da luz

A revisão independente apontou que o X analítico dos casos T06, T10 e T16 não existia: só havia estados temporais e
cores constantes. Esta entrega o acrescenta **sem mudar fórmula, tolerância nem fixture legado** (o diff de
`scripts/pilot-dissolve-corpus.py` acrescenta fontes, leituras do destino, seção analítica e contagens, e só reescreve 4 linhas de texto:
nota do head do plano, a premissa de `resolve-camera-clip.ts` e `HIDDEN_FALSE`, que agora diz ser contrato futuro).

**O que existe agora (e só isso):** a seção `analytic_probes` de `dissolve-cases.json`, `kind: origin-formula`, com 12
cenas sintéticas, 116 quadros e **451 sondas** de posição projetada e de cor de 8 bits, e 9 hipóteses (regras erradas).
Cada cena é de cor plana: planos coloridos, um marcador com canal `event`, lâmpadas com `flicker`, câmera que faz
`truckRight` a 0,36 unidade por quadro ou `hold`. As sondas ficam a ≥ 6 px de qualquer borda, em área plana.

| Cena | Prova |
| --- | --- |
| `pos-center-mixed`, `pos-end-mixed`, `pos-start-mixed` | instância presa com a câmera correndo (antes do corte) e câmera presa com a instância correndo (depois), os dois lados do corte, janelas `center`/`end`/`start` |
| `pos-edge-start` | o limite exato em que cada relógio passa a prender (primeiro quadro preso) |
| `pos-control-real`, `pos-control-held` | controles: sem hold (as hipóteses de grampo conjunto precisam **concordar**) e ambos presos |
| `lit-center-held`, `lit-end-held`, `lit-start-held` | luz no relógio de edição **real** f/fps com instância e câmera presas (o quadro preso não congela o flicker) |
| `lit-center-instance-running` | cena com `instance_start`/`camera_in` ≠ início do plano: relógio de edição distinto do relógio do clipe, instância correndo |
| `lit-ntsc-center` | fps racional 30000/1001 com a janela em 1794..1806 (59,8598 s contra 59,8 s nominais) |
| `both-center-mixed` | posição e luz juntas |

**Mutantes e hipóteses.** Pinagem conjunta dos relógios, instância/câmera trocadas, vizinho nos relógios do primário,
decks trocados na mistura, luz no relógio da instância, no relativo ao plano, no do clipe de câmera, no relógio de edição
limitado e no fps nominal. Cada uma é uma regra errada com **limiar pré-registrado**: separação ≥ 4 códigos (a
tolerância é 1), ≥ 3 sondas em ≥ 2 quadros, controles ≤ 1 código. O gerador **recusa** (`ORACLE_ANALYTIC_*`) uma cena
sem poder de recusa em vez de passar calado.

**Independência.** Sem avaliador de transição de produto, sem quadro puro fabricado para o hold misto, sem Rust nem GLSL como
referência; o módulo só importa `math` e `fractions`. O destino foi lido, nunca executado (`destination_reads`: 10 linhas
de `runtime/src/directed-scene.js`, `crates/core/src/scene_plan/build.rs` e `lighting.rs`, conferidas por presença
textual). A spec refaz tudo com meios próprios: `ctypes` para o hash de 32 bits, matriz de projeção do Three.js, forma
fechada da câmera, `Fraction` para pesos e `Decimal` de 60 dígitos para as curvas sRGB.

**Resultados (Windows x64, Python 3.13.17, sem Cargo, Node, GPU nem FFmpeg):**

| Comando | Saída |
| --- | --- |
| `generate --check` | exit 0, byte a byte, nada escrito |
| `verify` / `-O verify` | exit 0, 33 fontes, offline |
| `verify --origin E:/www/tesselapp` | exit 0, com os avisos `PIN_BLOB_UNVERIFIED` para os dois blobs ausentes |
| `verify --require-pin` sem origem | exit **4** |
| `verify --require-pin --origin …` | exit **4** (`PIN_UNVERIFIED`: o commit do pin não existe neste clone) |
| `digests` sem `--origin` | exit **2** |
| `pilot-dissolve-corpus.spec.py` e `-O`, offline | 167 testes, **OK (skipped=2)** |
| `pilot-dissolve-corpus.spec.py` e `-O`, com `TESSEL_ORIGIN_PATH` | 167 testes, **OK** (os 2 de origem real rodam; os testes do pin usam Git real, sem mock de `check_origin`) |
| `pilot_dissolve_analytic.spec.py` e `-O` | 36 testes, **OK** |

Evidência de força: (a) o avaliador independente da spec recalcula as 451 sondas (cor e cobertura), estados, luzes,
marcadores e **os conjuntos de hipóteses que cada sonda rejeita**; (b) um portão de consumidor (1/255 por canal e
\|média Δ\| ≤ 0,25) aplicado a produtos falsos falha para toda hipótese que a cena exige rejeitar, passa para os controles
e para a verdade, e recusa por **viés** um produto que trunca em vez de arredondar nas 12 cenas (média −0,45 a −0,89);
(c) 19 mutantes do próprio oráculo (as regras erradas dentro dele, progresso sem `+0,5`, profundidade do flicker, deslocamento
aritmético no hash, câmera um quadro adiantada, evento terminando antes, raio pelo canto do pixel, fila do pintor invertida,
luz em sRGB, fps nominal, sem queda de luz) mudam a seção ou fazem o gerador recusar, mais fov e viewport; os de fórmula também
reprovam os vetores à mão (flicker nos quatro valores que o teste Rust de `lighting.rs` registra, projeções a 50 dígitos,
relógios contados no papel, cor iluminada 138/100/239).

Hashes desta revisão (a versão original, §6, e a atualização do coordenador permanecem como histórico: o corpus anterior,
`865c4617ebca6ce2d83d67cbf5367cbb03d0c59ae93181d653613e0df7604f1b`, 206 891 bytes):

```text
e5f40f70ccdb069196224d45c8df09cbaec7539eedc06ea350e65de7966cd011  dissolve-cases.json     539508 bytes
5368450f0df266a5d4375adafcd73a82ccf9bc9d519ff2138ae0dc1129e644dd  dissolve-manifest.json   27892 bytes
```

**Esclarecimento de T06, T10 e T16 (o X documental):**

- **Existem agora** sondas analíticas de posição projetada e cor de 8 bits em planos **planos**, com hold misto e com o
  relógio real da luz (T06 fps racional e relógio do vizinho; T10 hold por instância e por câmera; T16 hold misto).
- **Quadros texturizados só se tornam reais na S2**, quando houver runtime que os desenhe: nenhuma amostra de textura existe
  ou é prometida na S0.
- **Nenhuma superfície de Rust, JSONL, UI ou motor** foi criada; o portão `FX_TRANSITION_NOT_YET` continua fechado até a S2A.
- As sondas são **oráculo `origin-formula`** de uma cena sintética; nada foi renderizado, medido ou executado.
- **`hidden`** é contrato **futuro** do destino: transições aceitam `hidden: true` ou ausente, e `hidden: false` explícito é recusa
  estrutural na S1a, independente de `explain`. O importador Rust de hoje **não** faz isso.

**Incorporação do plano final** (fonte `002fe9f`, integrado na `main` por `1796107`, contido em `58157ab`): o texto das seções
7.3, D14, D15, D17, D19, T06, T10, T16 e §17 foi lido e incorporado à seção analítica e a estes documentos. É leitura de
documento, **não prova**: nada do plano final foi executado ou validado aqui.

**Objeções restantes (nenhuma foi fechada por esta entrega):** a referência é analítica e sintética, **não** é golden da origem
(`ORIGIN_GOLDEN_NOT_COMPARABLE`) nem execução dela (`ORIGIN_NOT_EXECUTED`); cenas de cor plana não provam amostragem de
textura nem filtro; o `--check` em outro host (libm do `pow`) continua sem rodar no Mac para esta revisão; o commit do pin
`d40ac3f7c0` segue ausente neste clone; as constantes do flicker vêm do texto da origem e do teste Rust, não de execução do JS;
os limiares são pré-registrados e **A validar** pela revisão de design; sem GPU, FPS ou desempenho (o lock não foi tomado).

**Linhas propostas ao coordenador (não editei `backlog.md` nem `features/README.md`):**

- **BL082.4.1:** manter **A validar**. Acrescentar à evidência `scripts/pilot_dissolve_analytic.py` e `.spec.py` (36 testes),
  a seção `analytic_probes` (12 cenas, 451 sondas), 33 fontes e este §10; reabrir o fechamento de S0 só depois da revisão
  independente do P2-2.
- **F064** (matriz): manter lib/JSONL/UI **Pending**; coluna de oráculo: "Python `origin-formula` com X analítico de hold misto e
  luz real (planos planos), quadros texturizados só na S2"; link [fx-transitions.md](../features/fx-transitions.md).
