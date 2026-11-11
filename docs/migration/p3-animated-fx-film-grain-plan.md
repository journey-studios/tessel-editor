# P3 — parâmetros animados, easing, seeds e filmGrain

Data: 2026-11-10. Autorizado pelo usuário após P2: mergear a PR39, preparar um plano detalhado e encaminhar ao Cursor Sonnet 5.5 para implementar a próxima entrega.

## 1. Base, objetivo e fonte de status

- P2 foi integrado por squash na PR39: `e18b0cf39f5b94c71640ad397f3f21bc9781666f`, head revisado `d1f5349`, em 2026-11-10T17:14:28Z. Mensagem do squash sem trailer de assistente.
- Atualizar `origin/main` ao iniciar; a base mínima desta entrega é esse merge. Não implementar sobre a branch antiga `docs/reconcile-migration-status`.
- O catálogo entregue tem `vignette` e `fade`; o inspector F057 é compartilhado entre core, JSONL e GPUI. P3 acrescenta animação desses parâmetros e um terceiro look real: `filmGrain`.
- A fila, os estados e os responsáveis vivem somente em `docs/backlog.md`. Este documento define contrato, sequência e critérios. Usar BL082.2 para animação/seed, BL082.1 para o terceiro look, BL004/BL029/BL040 para o inspector e BL036/BL018/BL019 para proveniência/equivalência.
- Não considerar históricos de disco bloqueado ou testes de heads antigos como estado atual. P2: suíte2060/0/12 e janela153/153 em5eb577b; o ajuste final de diagnóstico d1f5349 tem regressão e gates próprios. Nenhum deles valida automaticamente P3.

### Resultado que a pessoa deve conseguir usar

1. Criar um fade, adicionar chaves de quantidade, escolher easing e ver a animação ao fazer seek/play.
2. Criar/mover/remover chaves de cor e centro da vinheta sem perder os outros componentes nem confirmar um candidato antigo.
3. Criar filmGrain, ajustar quantidade/tamanho/resposta de luminância/cor e definir seed reproduzível.
4. Rever o mesmo quadro após seek, cancelar um gesto, desfazer/refazer e salvar/reabrir com os mesmos valores e pixels.
5. Executar as mesmas operações pela lib, pelo processo JSONL sem janela e pela UI semântica; exportar o resultado.

## 2. Escopo autorizado e fronteiras

### Dentro de P3

- Modelo tipado de chaves de parâmetros de FX, parser, validação, resolução por quadro e edição atômica.
- Easing do piloto, hold antes/depois das chaves, leitura dos valores exatos nos frames de chave.
- Valores numéricos, cores, pontos e booleanos; o booleano é necessário ao `colored` de filmGrain.
- Seed explícita ou derivada do ID e hash inteiro determinístico; vetor de equivalência Rust/referência independente/GLSL.
- Edição e gestos ordenados de chaves no core, sem depender de ponteiro, GPUI ou socket.
- Controles reais de chaves no inspector, com equivalentes semânticos; não é a timeline dirigida inteira.
- filmGrain completo no catálogo, inspector, plano, renderer, monitor e export.
- Persistência, pacote/restauração e undo/redo dos dados novos, com gates e contratos atualizados.

### Fora de P3

- Escopos shot/node, transições, crossDissolve, segundo deck, ripple/roll/split/slip, timeline dirigida completa P6.
- Outros 56 efeitos, presets vazios, rig/path/clima/luz/narração, serviços/cloud/ML, refatoração geral e nova plataforma.
- Produção/reedição do filme privado, criação de áudio, aceite humano do filme, promessa de GPU real/Windows/ponteiro físico sem prova específica.
- Trocar a política de último candidato legal de P0/P1 ou alterar o livro/oráculos/tolerâncias S10 para obter PASS.

## 3. Referência primária e levantamento obrigatório

Origem somente leitura: snapshot `d40ac3f7c0` no repo JS, inicialmente em `/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f`. Usar `git show`/`cat-file`, sem alterar, instalar, executar o piloto ou copiar mídia privada.

| Fonte no snapshot JS | O que conferir e fixar no manifesto |
|---|---|
| `src/fx/fx-schema.ts` e specs | Forma de keys, seed, tipos e limites |
| `src/fx/fx-keys.ts` | Ordem, frames inteiros locais e range da duração |
| `src/keyframes/keyframe-rules.ts` e specs | Easing válido, substituição de chave e preservação de easing omitido |
| `src/keyframes/resolve-keyframes.ts` e specs | Easing na chave de chegada, bordas, leitura exata e hold |
| `src/camera/easing.ts` | `linear`, `easeInOutCubic`, `easeOutCubic` e curvas exatas |
| `src/fx/fx-command-keys.ts` | keyParam/unkeyParam, setParam estático com animação presente |
| `src/fx/fx-command-step.ts` e handlers de duração | Campos preservados, notas, limpeza de keys vazias e comportamento de edição |
| `src/fx/fx-clip-sample.ts` | Interpolação por tipo, envelope e seedFor |
| `src/fx/fx-hash.ts`, `resolve-fx-frame-hashes.spec.ts` | FNV UTF-8, lowbias32, vetores e overflow uint32 |
| `src/fx/catalog/filmGrain.ts` | Parâmetros, defaults, unidades, booleano, reseed e labels |
| `src/runtime/fx/shaders/filmGrain.ts` | Espaço de cor, tamanho da célula, resposta e composição |
| `src/runtime/fx/shaders/chunks/hash.ts`, color/params/prelude | HashPixel, triangular, constantes, conversões e escala de pixels |
| Specs de keys/gestos/retime do piloto | Limites de movimento, ordem e semântica das propostas |

Os caminhos da tabela são relativos a `packages/scene-engine/`. Reabrir os arquivos efetivos; o inventário não substitui o código. Se um objeto faltar, declarar o bloqueio de equivalência; não inventar goldens.

### Arquitetura Rust a reaproveitar

- `crates/core/src/directed_scene/fx/`: catálogo, valores, import, resolução, controle e referências puras.
- `scene_edit/fx.rs` e `fx_inspector.rs`: operações, atomicidade, campos, reset, oferta, labels e probe.
- `scene_project/fx*.rs`, `ops.rs`: consultas, parser, comandos e gesto já compartilhados.
- `scene_interaction`, histórico e conversão de tempo já existentes; localizar os módulos reais, sem presumir um arquivo novo a partir do nome desta seção.
- `scene_plan/` e PlanTimeline: plano por quadro, validação e capabilities.
- `desktop/workstation/scene_fx*.rs`: adapter GPUI e testes, sem regras de interpolação/retime.
- `runtime/src/output-fx.js`: materiais retidos, uniforms e dois targets sem depth. Não recriar material/shader a cada quadro.
- Reaproveitar `directed_scene::apply_easing` quando os contratos forem iguais; não mudar os easings da câmera por conveniência de FX.

## 4. Contrato de dados e invariantes

### 4.1 Modelo canônico

- `fx` continua sendo uma chave do bundle dirigido. Não criar cópia de animação na sequência do projeto ou no estado GPUI.
- `params` conserva os valores estáticos/base; `keys` associa o ID de um parâmetro a uma lista ordenada de `{frame, value, easing}`.
- Frame de chave é inteiro **local ao clipe FX**, em `[0, durationFrames)`. Não usar frame do bundle ou da sequência como se fossem locais.
- Uma curva tem frames estritamente crescentes, sem duplicatas; keys de parâmetro inexistente e valores com tipo/faixa errados são recusados.
- O import valida a entrada recebida: não ordenar ou eliminar duplicatas silenciosamente. Comandos de inserção montam a ordem válida.
- Confirmar no código do piloto a política para listas vazias e mapas vazios; documentar a normalização escolhida. No-op e lote líquido neutro não devem apagar uma camada autorada nem criar undo.
- Preservar os campos desconhecidos do documento de acordo com o contrato lossless existente.
- Registrar limites concretos e agregados de chaves por curva/clipe/documento antes de habilitar o parser; rejeitar excesso com código/pointer, sem alocar sem limite. Não confundir esse limite de dados com o budget de passes8, que continua apenas aviso.
- Inteiros extremos, NaN/Inf nas APIs Rust, arrays gigantes e operações de frame devem falhar sem panic, wrap, truncamento ou alteração parcial.

### 4.2 Compatibilidade

- Bundles antigos sem keys e seeds devem conservar estado, planos e pixels dos dois looks atuais.
- Seed já é lida e preservada como Option<u32> no Rust; antes não alterava os dois efeitos existentes. Não chamá-la de campo totalmente novo nem materializar um default no documento só ao consultar/renderizar.
- Separar versão do documento/catálogo e capacidade do renderer. Se houver mudança de versão, escrever decisão, compatibilidade e teste; não fazer migração silenciosa de projetos existentes.
- filmGrain pode exigir uma capability específica de ruído/seed, além de outputFx. Testar um runtime P2 sem essa capacidade: falha tipada na negociação/preflight, sem desenho parcial ou alegação de suporte.
- Preservar os erros de scopes/transitions não entregues. Remover FX_KEYS_NOT_YET somente dos caminhos agora implementados.

## 5. Avaliação temporal — decisões obrigatórias

### 5.1 Regras do piloto a preservar

1. Na ausência de keys, usar `params`, senão o default do catálogo.
2. Com keys, antes da primeira segurar seu valor; depois da última segurar o último. Não usar a base antes da primeira chave.
3. No frame exato de uma chave, retornar seu valor armazenado diretamente; não reconstruí-lo por interpolação em t=1.
4. Entre chaves, easing pertence à **chave de chegada**. O easing da primeira não afeta um intervalo anterior inexistente.
5. Os três nomes iniciais são `linear`, `easeInOutCubic`, `easeOutCubic`. Não acrescentar hold/step como easing inventado: booleanos já têm mudança discreta pelo tipo.
6. Números interpolam linearmente após easing. Cores interpolam canais em luz linear; pontos interpolam componentes.
7. Booleanos mantêm o valor anterior até o frame da próxima chave, quando assumem exatamente o novo valor; easing não cria frações booleanas.
8. Envelope é o existente, amostrado no meio do frame. Resolver intensidade animada antes de calcular `amount = clamp(intensity × weight)`; não multiplicar peso/intensidade duas vezes.
9. Nada depende da ordem dos pedidos, de frames anteriores, relógio ou cache mutável de RNG.

### 5.2 Tempo e edição estrutural mínima

- Converter a posição exibida pelo monitor pela regra existente do core: tempo da sequência → frame da edição da cena → frame local do FX. O adapter não faz divisão por fps aproximado.
- Testar rates racionais, cena posicionada fora do zero e conversões com speed50/100/200 quando aplicáveis ao caminho existente.
- Mover o início do FX desloca sua animação no tempo de edição; os frames locais das keys ficam iguais.
- Reduzir duração com key fora do novo intervalo deve recusar atomicamente, sem apagar/mover keys em silêncio. Uma edição explícita que retime keys e duração no mesmo lote pode ser válida e cria um undo.
- Não alterar source_start, câmera ou duração de shots. Se uma operação existente significar outra coisa, registrar o contrato real e a divergência antes de mudar comportamento.

## 6. Operações públicas e transações

Nomes abaixo são a proposta para o parser FX; o executor pode ajustá-los para consistência com APIs existentes, documentando a tabela final. Nenhuma mudança de regra fica só no parser.

| Operação | Intenção e contrato |
|---|---|
| `key_param {clip,param,frame,value,easing?}` | Inserir em ordem ou substituir nesse frame. Chave nova usa linear se easing omitido; chave existente preserva seu easing quando omitido e respeita um easing explícito |
| `unkey_param {clip,param,frame}` | Remover somente a chave indicada. Chave inexistente é erro tipado. Ao remover a última, revelar novamente a base estática |
| `set_key_easing {clip,param,frame,easing}` | Alterar a chave exata; não regravar as outras nem trocar a identidade pelo índice da lista |
| `move_key {clip,param,from_frame,to_frame}` | Mover a chave sem cruzar vizinhos nem colidir. API numérica recusa; gesto fornece limites e mantém último candidato legal |
| `shift_keys {clip,param,delta_frames}` | Deslocar uma curva inteira com checked arithmetic e uma entrada de undo; preservar ordem/valores/easings |
| `scale_keys {clip,param,pivot_frame,factor}` | Escala positiva em torno de pivô, regra de arredondamento inteira documentada; colisão/fora do clipe é recusa atômica, nunca deduplicação |
| `set_seed {clip,seed:uint32\|null}` | Seed explícita ou voltar à derivação por ID; sem RNG. Mesma seed é no-op; null não congela uma seed derivada no documento |

### Regras compartilhadas de edição

- `set_param` continua editando a base estática e preservando keys. Quando houver curva, retornar aviso/metadata explícitos de que as keys prevalecem, como no piloto.
- Chaves de cor/ponto guardam o valor tipado inteiro do parâmetro; não criar curvas implícitas por canal incompatíveis com o schema. Editar um componente de uma chave preserva os irmãos dessa chave, não os valores base nem de outra chave.
- O reset de parâmetros existente preserva a animação; não apagar keys nem seed por um reset visual. Explicar base versus valor avaliado no modelo/labels. Remover animação é ação explícita, atômica e com undo, por lote de unkey ou comando equivalente.
- Não inferir captura de uma key da seleção atual no callback de blur. O alvo inclui projeto, cena, clipe, parâmetro, frame de chave e revisão de origem.
- Uma operação válida altera somente seu alvo; recusa deixa documento, revisão, histórico, seed e cache sem candidatos confirmados.
- Lote com alterações que se anulam não cria undo. dry-run/probe/status/inspect/frame não gravam nada.
- Regra de captura: nova chave no frame atual usa o valor avaliado do core, salvo valor explícito do chamador. Capturar sobre chave existente não redefine easing por omissão.
- Emitir resultado com alvo/frame anterior e novo, avisos, códigos estáveis e hints úteis. IDs de nó que incluem frame devem ser atualizados após um move sem atingir outra chave por engano.

### Gestos

- Reaproveitar sessão/candidato do core. begin captura base; cada update deriva dela, não do update anterior.
- begin/update/commit/cancel/status têm equivalentes headless. Um drag repetido cria só um undo no commit; Escape/cancel/troca de cena/projeto/revisão/undo externo não confirma nada antigo.
- Proposta inválida mantém o último candidato legal, com recusa consultável. O monitor volta ao documento quando o gesto deixa de ser live.
- Para mover keys, limites de ordem/range vêm do core. UI só traduz a proposta e pinta a resposta.
- Testar mistura indevida de verbos (gesture FX de valor × movimento de key), alvo de outra curva e resultado tardio de sessão anterior.

## 7. Inspector, JSONL e UI GPUI

### Modelo neutro

- Estender F057 com tipo booleano, controle sugerido, base_value/valor avaliado, animated, keys, key_at_frame, easing, operações legais e motivos de disabled.
- Manter compatibilidade do `value` estático de P2; adicionar o valor amostrado explicitamente em vez de mudar seu significado silenciosamente.
- Quando um parâmetro estiver animado, o editor de base fica identificado como base. Para esta entrega, desabilitar sua edição pela UI com motivo e oferecer os editores de keys; `set_param` explícito da API continua disponível com aviso. Não criar auto-key implícito.
- Para booleanos usar Switch real do Kit; os controles não podem aceitar números 0/1 como se fossem booleanos.
- Defaults, faixas, passos, labels pt-br/en/es, oferta, reset e validação continuam no core. A UI não lista controles por nome do efeito.
- Consultas distinguem documento/candidato, base/valor avaliado e frame local/bundle. `probe` julga contra o mesmo source escolhido e não é restringido por um filtro de resposta de outro clipe.
- `reset.op` por campo e reset agregado preservam a regra multicomponente de P2. Em consulta de candidato, indicar que os ops devem ir para gesture.update para não confirmar valores provisórios por engano.

### Superfícies mínimas na janela

1. Selecionar FX e parâmetro e ver base, valor no frame atual e indicação de animação.
2. Adicionar/capturar key no frame exibido; navegar anterior/próxima pelo core; remover key selecionada e remover animação explicitamente.
3. Lista/accordion com keys e editores de frame, valor tipado/componentes e easing; ordenação vem do core. Scroll/limites visuais para curvas grandes.
4. Movimento de key por controle semântico e por gesto do adapter compartilhado; mover/retimar a curva sem construir uma timeline P6 completa.
5. Campo de seed uint32 e ação de voltar à automática, mostrando a seed efetiva só como leitura; controle real para `colored`.
6. Play/seek mostram valores e pixels resolvidos, sem materializar alterações no documento. Budget inclui o terceiro efeito.

Todo controle tem Command, nó vivo, parser, handler comum e testes. Propor IDs estáveis como `scene.fx.key:<clip>:<param>:<frame>`; vincular a revisão evita reaplicar uma chave antiga depois de move/undo. Índice de linha não é identidade. Widgets são do Kit, tokens do tema, um só monitor; diálogo novo exige has_dialog. Não operar por coordenadas.

## 8. FilmGrain — especificação do terceiro look

### Catálogo da origem

| Parâmetro | Tipo | Faixa/default/passo |
|---|---|---|
| amount | number | 0..1 /0.25 /0.01 |
| size | number | 0.5..4 /1.4 /0.1; px declarados a720p |
| luma | number | 0..1 /0.7 /0.01 |
| colored | boolean | false |
| intensity | number comum do catálogo | 0..1 /1 /0.01; confirmar injeção/ordem no meta da origem |

Família look, pass, typicalDurationFrames96, reseed per-frame, needs seed. Só entra na oferta quando o desenho real e todas as superfícies funcionarem.

### Seeds e hash

- Seed base = explícita ou FNV-1a dos bytes UTF-8 do ID; FNV não é sobre code units UTF-16.
- Para filmGrain, seed efetiva = lowbias32(base XOR frameInClip), com coerção/wrap uint32 igual ao JS. Comparar casos que atravessam a fronteira de32bits; não truncar inteiros sem contrato.
- HashPixel: lowbias32(lowbias32(seed XOR0x9e3779b9) + x + (y <<16)), sempre com wrap uint32.
- lowbias32 usa XOR/shifts e multiplicadores0x7feb352d/0x846ca68b. Rust usa wrapping_*; GLSL usa uint, nunca hash por float/sin/clock.
- unitFloat extrai24bits por shift8/divisão16777216; triangular soma duas amostras com lowbias32(hash XOR0x68bc21eb) e subtrai1.
- Os vetores inteiros devem coincidir exatamente. Tolerância de pixels não pode esconder erro de seed/hash.

### Pixels

- Tamanho de célula efetivo = max(size × height/720,1), aplicado uma única vez no desenho. Coordenadas de gl_FragCoord têm origem embaixo; a referência CPU precisa converter a leitura RGBA de cima para baixo explicitamente.
- mono = triangular(hashPixel(seed,cell)); colored usa os hashes deslocados0x632be5ab e0x85157af5 nos outros canais, como o shader fixado.
- O ruído é somado no espaço perceptual sRGB, com resposta de luminância e clamp, e então volta à luz linear para mistura pelo amount do passe. Não aplicar o ruído diretamente em linear nem aplicar intensidade duas vezes.
- Confirmar no chunk de cor a função exata de luma; transcrever a partir da fonte, sem assumir coeficientes por memória.
- amount/intensity/weight0 é identidade exata em bytes no caminho existente. Mesmo frame/seed/dimensões dá os mesmos pixels após seek e reabertura. Outro frame/seed precisa mudar uma região mensurável em caso discriminante.
- Preservar o alpha da fonte conforme o contrato do editor e declarar divergência do piloto que escreve alpha1; não alterar vinheta/fade para seguir esse detalhe da origem.
- Estudar o material GLSL e suporte a uint/WebGL2 antes de escolher a integração. Se o grain precisar GLSL3, confinar a mudança e testar o runtime real; não quebrar shaders dos dois looks existentes.
- Reusar materiais/geometry/targets. Nenhuma compilação de shader ou alocação de alvo por passe a cada quadro.

## 9. Plano, cache e ciclo de vida

- Core resolve keys, valores, peso e seed efetiva. Runtime recebe o passe já resolvido e desenha; não resolve easing/chaves/reseed em JS.
- Validar no contrato do plano tipo/faixa dos novos valores, booleano e uint32; seed ausente/adulterada/float/negativa deve ser recusa tipada quando exigida pelo efeito.
- PlanTimeline precisa equivaler a ScenePlan para qualquer ordem de frames e depois de editar keys/seed. Invalidação por revisão/candidato, não cache de um frame aplicado a outro.
- Retenção, resize, perda/restauração de contexto, falha de passe e retain:false conservam o contrato de cleanup. Após uma recusa o próximo frame válido precisa desenhar.
- Não ordenar, clonar ou revalidar a curva inteira em cada frame. Curvas tipadas já validadas e busca limitada por frame devem reutilizar o preparo; memória retida é função das curvas/cenas, não do total de frames pedidos. Se uma estrutura nova de cache for necessária, provar invalidação por revisão e limite de memória.
- Testar combinações vinheta/fade/grain em ordens distintas, passes repetidos e budget>8. A ordem deve ser discriminante, não apenas comparar duas imagens iguais de operações comutativas.
- Jobs/export congelam o documento e os recursos como já fazem; edição durante um export não pode mudar keys/seed do snapshot em execução. Não mover render/FFmpeg para a thread UI/core de domínio.

## 10. Corpus e evidência independente

Criar casos pequenos P3 e manifesto com commit/blob SHA e hashes. Não copiar mídia privada ou usar o filme real para esconder ausência de fixture.

| Grupo | Casos mínimos |
|---|---|
| Interpolação | Antes/na/depois da primeira/última key, todos os easings, intervalo1frame, chave substituída, valor decimal exato no frame da key |
| Tipos | Número, cor/ponto por componentes, booleano no limite exato, tipo incompatível e parâmetro inexistente |
| Range/ordem | Frame negativo/fracionário/duração, duplicatas/desordem, keys fora após resize, overflow, orçamento de dados |
| Tempo | FX com início diferente de0, cena em sequência deslocada, NTSC e conversão de speed quando aplicável |
| Edição | Preservar easing omitido, set estático com keys, removerúltima, lote neutro, dry-run/probe/reset, undo/redo/reabrir |
| Gestos | Move entre vizinhos, limites/collision, shift/scale com arredondamento, último candidato legal, cancel/foco/revisão/cena/projeto |
| Seeds | Base ausente/0/maxu32/null, IDs UTF-8 nos testes puros do hash, troca de ID, frame0/1/repetido/distante e fronteira32bits |
| Pixels | Grain mono/color, tamanhos e resoluções/formatos distintos, sombras/meios-tons/highlights, alpha, zeros e ordem de composição |
| Runtime | Falso runtime P2 sem capacidade, shader compile, resize/contexto/falha/recuperação, programas/alvos retidos |
| Export | Frames de keys e bordas, grain determinístico fora de ordem, snapshot de keys/seed, pacote restaurado sem origem |

- Referência de pixels e hash deve ser independente das funções sob teste. Não gerar expected chamando a própria implementação Rust/GLSL.
- Inteiros: igualdade exata Rust/referência/GLSL em vetores. Pixels: thresholds pequenos e justificados antes do resultado; incluir controle negativo/mutações para seed incorreta, hash float, easing da chave errada, mistura em cor errada e peso duplicado.
- Comparar pixels em configurações equivalentes (dimensões, seed, frame, cor e qualidade). Preview proxy com outra resolução tem outra grade de células; documentar a diferença e não prometer igualdade byte a byte com export de tamanho distinto.
- Se a origem não for executada, continuar usando origin-formula, ORIGIN_NOT_EXECUTED e ORIGIN_GOLDEN_NOT_COMPARABLE. Texto pinado/verificador não equivale a executar o motor original.
- Não modificar manifest/tolerância/corpus S10. Os corpora P0/P2 devem continuar verificando sem alteração de expectativas.

## 11. Sequência de implementação e checkpoints

| Etapa | Entrega concreta antes de avançar | Prova mínima |
|---|---|---|
| A | Base/ownership, fontes fixadas, decisão de schema/limites/capability e tabela de comandos finais | Manifest, plano atualizado e smoke focado P2 |
| B | Tipos, importação e validação de keys/booleano/seed | API pura e erros com estado intacto; core sem GPUI |
| C | Resolvedor temporal e hashes | Vetores independentes, frames exatos, easing chegada, bool e uint32 |
| D | Operações, retime, transação e gesto | Lib/JSONL sem janela; histórico, no-op, revisão e recusas |
| E | Inspector e controles de keys/seed/booleano | GPUI/nós/parser; base versus avaliado e operações reais |
| F | Grain no catálogo/plano/runtime e capabilities | Pixels GL reais, hash inteiro, retenção e recuperação |
| G | Persistência, pacote, monitor e export | Projetos sintéticos, instância privada, JSONL processo real e exportdecodificado |
| H | Gates finais, docs/matriz/catálogo e PR | Evidência do código final e patch do backlog ao coordenador |

É uma tarefa sequencial com commits focados. Não entregar só B/C e declarar P3 pronto. Caso exista um impedimento real, entregar o que está correto com estados I/A validar e erro reproduzível, sem habilitar o restante.

## 12. Gates finais e recursos

1. `cargo fmt --all -- --check`.
2. `./scripts/cargo check --workspace --locked`.
3. `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`.
4. `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked` com lock de GPU para as provas reais.
5. `./scripts/cargo build -p tessel-editor --locked` e árvore do core sem gpui/wry.
6. Runtime Node pertinente/completo conforme mudanças, specs Python normal/-O, corpora novos e regressão P0/P2.
7. Prova semântica da janela no binário/runtime finais: key edits/seek/preview/commit/cancel/undo/reopen/budget/seed/booleano e export.
8. Smoke de interação FX+histórico para não corromper as sessões/segmentos ao trocar cena ou cancelar gesto.

Registrar comando, exit code, código testado, hash do binário/runtime e casos não executados. Não repetir todos os gates após docs-only; não usar execução antiga como prova de código novo. Um ajuste posterior isolado exige rechecagem proporcional e descrição da camada coberta.

### Política atual de Cargo e disco (obrigatória para todos os scripts usados)

- Cargo sob mbx, preferencialmente wrapper do projeto quando ele respeitar a política. O target é o padrão gerenciado.
- Nunca definir CARGO_TARGET_DIR, --target-dir, build.target-dir ou TESSEL_CARGO_LANE, inclusive em helpers de prova. Inspecionar scripts; se mbx falhar, avisar e registrar sem contornar.
- Sem build/cache/artefato temporário maior que aproximadamente500MB em /tmp/private-tmp. Saídas grandes vão para diretório próprio foraGit e são removidas ao terminar, preservando logs pequenos.
- Não executar clean/gc manual, apagar caches/traces/dados alheios ou encerrar processos externos. Limpeza automática do mbx é gerenciada por ele.
- Reserva16GiB antes de iniciar pesado, piso12GiB; verificar entre fases. Builds/GPU devem ser serializados e respeitar o lock compartilhado, inclusive diante de benchmarks de outra frente.
- Testes de UI somente scripts/ui e handlers semânticos. Screenshot nativo é observação, não operação por coordenada. Não usar a instância/projeto/socket do usuário.

## 13. Documentação, ownership e entrega

- Skill tessel-feature-contract obrigatória; atualizar F016/F057 e matriz na mesma entrega, além de contrato de keys/hash se o módulo concreto justificar um documento próprio.
- Atualizar catálogo/auditoria de migração só com prova: grain pode levar a3de59, não paridade integral. A animação é capacidade transversal, não mais dezenas de efeitos entregues.
- Coordenador mantém o backlog sujo da raiz; o filho lê a cópia atual apenas para contexto e entrega patch de suas linhas. Reutilizar IDs, não atribuir famílias futuras.
- Ownership do implementador: módulos FX/keys/hash/inspector/plano/adapters/runtime/tests/docs necessários ao P3, na sua branch. Não editar raiz, origem JS, filme privado, gizmos de outra frente ou arquivos de plataforma sem causa comprovada.
- Copiar este plano para a branch da implementação e atualizar suas decisões/evidências sem criar uma segunda fila de status.
- Um Cursor Sonnet5.5 em checkout T3 isolado, sem subagentes. Branch nova da main atual; não fazer checkout/worktree manual no shell para outro agente.
- Commits pela skill commit, sem referências/trailers de assistentes. Conferir o corpo real antes de publicar.
- Criar/push PR da entrega autorizado, com link_pull_request como chamada seguinte; não mergear, publicar, monitorar ou fazer polling pelo filho. Coordenador revisa e integra.
- Retorno: PR/head/base, comandos realmente executados, resultados por superfície, hashes, paths de evidências pequenas, divergências da origem, pendências e patch do backlog. Última linha PR URL, head SHA, checks/reviews.

## 14. Definição de pronto

P3 está entregue quando keys/easing/seed e grain funcionam e são editáveis pelas três superfícies, persistem/desfazem, apresentam valores/pixels corretos no monitor e export e passam os gates do código final. Nenhum controle habilitado representa uma operação ausente. Ausência de hardware/golden/ponteiro mantém limites explícitos; ausência da prova funcional de uma superfície mantém a entrega A validar.

## 15. Decisões e estado da implementação (atualizado pelo implementador)

Registro do que foi decidido e provado; o status e a fila vivem em `docs/backlog.md` (patch entregue ao coordenador).
Contratos finais: [F058](../features/fx-keys.md) (chaves/seed/inspector) e [F059](../features/film-grain.md) (grão).

**Tabela final de operações** (`scene.fx.edit`): `key_param`, `set_key_easing`, `remove_key`, `move_key`, `shift_keys`,
`scale_keys {anchor,num,den}`, `clear_keys`, `set_seed`. Diferenças em relação à proposta da seção 6: `unkey_param` ficou
`remove_key`; `set_seed` recebe `seed` (`null` volta à derivação); `scale_keys` usa âncora e fração racional (`num/den`) em
vez de `pivot_frame`/`factor` (aritmética exata, meio para cima).

**Decisões**

- Versão do catálogo continua 1: `keys` e `seed` já eram campos do formato; sem migração silenciosa. Nova capacidade do
  runtime `fxNoise` (11ª), derivada só de passes com seed; runtime P2 recusado na negociação.
- Limites de dados: 256 chaves por curva, 1024 por clipe, 16 384 por camada (`FX_LIMIT`); o aviso de 8 passes segue só avisando.
- Seed explícita nunca é materializada ao consultar; `null` não congela a derivada.
- Não há auto-chave: sliders editam a base; captura grava o valor **amostrado** pelo core.
- Reduzir duração com chave fora do intervalo é recusa atômica; easing não escrito continua não escrito.
- O reset do visual preserva animação e seed; remover animação é `clear_keys`/`remove_animation` explícito.
- Alpha da cena preservado no grão (o piloto escreve 1). Texturas de origem já são sRGB codificado.

**Etapas A–H:** todas entregues e provadas (ver F058/F059): tipos e importação (B), resolvedor e hashes (C), operações,
transação e gesto (D), inspector e janela (E), grão no catálogo/plano/runtime (F), persistência/monitor/export/processo
real (G), gates e docs (H). Evidências por superfície, comandos e hashes no relatório de entrega e no rodapé de
`docs/features/README.md`.

**Limites que permanecem:** GPU nativa e Windows; execução do motor original (`ORIGIN_NOT_EXECUTED`,
`ORIGIN_GOLDEN_NOT_COMPARABLE`); ponteiro físico (BL043); 56 efeitos; escopos `shot`/`node` e transições (P4); timeline (P6);
preview em outra resolução tem outra grade de células do grão.

**Correção após a revisão da PR 42 (BL082.30–.34):** a UI das chaves foi completada contra a seção 7 (quadro e valor
tipados, mover, deslocar/escalar, navegação `scene.fx.seek` no core, seed arbitrária, `base_revision`, base desabilitada
sob chaves); `scale_keys` ficou exato para `i64::MIN`/`i64::MAX`; o lote tem orçamento (1024 ops) e os limites de dados
são conferidos antes de copiar; o gesto liga verbo e alvo; `key_param` compara tipos. Diferenças em relação ao plano:
o easing por chave é um botão que percorre os três nomes (não um `Select`), e velocidade/offset do clipe de cena são N/A.
Itens do coordenador (nits F4/F7/F9) não foram tocados. Estado e evidências por superfície em F058 e no backlog.
