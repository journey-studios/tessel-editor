# P4b-S0 — corpus e oráculo do cross dissolve (relatório)

Frente **BL082.4.1**, feature **F064** ([fx-transitions.md](../features/fx-transitions.md)). Plano:
[p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md) (v2). Esta entrega é só o **oráculo executável e o corpus**;
não há Rust, runtime, JS de produção, GPUI nem nó de árvore. A feature **não está implementada**.

## 1. O que foi entregue

| Arquivo | Papel |
| --- | --- |
| `scripts/pilot-dissolve-corpus.py` | Oráculo de referência (Python stdlib), gerador/verificador do corpus e do manifesto |
| `scripts/pilot-dissolve-corpus.spec.py` | 161 testes: vetores à mão, aritmética exata, mutantes, adulteração, CLI, verificador de origem |
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
| Fontes | 26: 21 do atestado do coordenador (19 idênticas à ponta, 2 diferem) + 5 só da ponta |
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
