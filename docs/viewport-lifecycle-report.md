# Ciclo de vida do preview sob incidente — frente C

Escopo: provar (e corrigir, se falhasse) que o monitor com proxy automático e refino **converge para a imagem original
correta** quando o ambiente falha, que o render/export original **não muda** e que **nada fica retido**. Contrato e
tabela de incidentes: [`docs/features/viewport.md`](features/viewport.md#ciclo-de-vida-do-preview-sob-incidente).

Resultado: **nenhum bug real reproduzido**. Todas as provas passaram contra o binário release já existente; não houve
mudança de Rust, de bridge nem de comportamento do runtime. A entrega é o conjunto de provas de regressão (JS + janela
real), um gancho de teste aditivo no `debug` do runtime e a documentação.

## Arquivos

| Arquivo | Tipo |
|---|---|
| `runtime/test/lifecycle.test.mjs` | novo, 8 testes (Chrome) |
| `runtime/test/lifecycle-fixtures.mjs` | novo, sequências original/proxy, gatilhos de perda, helpers |
| `runtime/src/index.js` | +7 linhas: `TesselRuntime.debug.glMemory()` (só leitura; texturas/geometrias/programas GL que o Three.js acha vivos) |
| `scripts/viewport-lifecycle-proof.py` | novo, harness na janela real (WKWebView) |
| `docs/features/viewport.md` | seção de ciclo de vida, linhas de evidência, contagem 53→61 |
| `docs/viewport-lifecycle-report.md` | este relatório |
| `docs/viewport-performance-next-report.md` | uma linha apontando que a lacuna "perda com proxy / ocultar-reabrir" foi fechada |

`runtime/dist` (gitignored) foi reconstruído com `pnpm run build` para incluir o gancho. Nada de Rust, `Cargo.*`,
matriz, `tesselapp` ou origem foi tocado; nenhum comando Git rodou.

## Comandos e resultados

| Comando | Resultado |
|---|---|
| `cd runtime && TESSEL_REQUIRE_RUNTIME=1 pnpm test` | **61/61** (53 anteriores + 8 novos), ~48 s |
| `python3 scripts/viewport-lifecycle-proof.py --out /tmp/c-lifecycle-final.json --shots /tmp/c-shots` | **147 PASS, 0 FAIL**, exit 0, ~50 s, renderer `Apple GPU`, 0,03 GiB de artefatos |

Os testes novos foram provados capazes de falhar: mutações temporárias no `assets.js` fizeram os testes correspondentes falharem; a fonte foi restaurada e o `git diff` de `runtime/src` mostra só o gancho.

### O que cada prova cobre

**`lifecycle.test.mjs` (Chrome, contexto WebGL real, perda real por `WEBGL_lose_context`):**

1. perda no meio de um quadro de scrub com proxy: `context_lost`; o mesmo proxy e o mesmo original depois da restauração;
   nenhuma textura GL vazada; `proxy_failed` 0;
2. quadros pedidos durante a perda são recusados como `context_lost` sem derrubar o proxy;
3. o refino depois da perda converge ao original byte a byte;
4. cinco perdas seguidas: recursos limitados, `gl_textures` estável por redesenho, `context_generation` +ciclos;
5. asset trocado durante a perda: sem proxy velho, pixels do novo, `release` zera (`glMemory().textures` 0);
6. quadro cancelado em voo não deixa nada e o próximo é exato;
7. projeto trocado com quadro em voo: o antigo é cancelado, o novo desenha os próprios pixels;
8. `dispose` com quadros de proxy e quadro em voo libera tudo e fecha o runtime.

**`viewport-lifecycle-proof.py` (janela real, HOME isolado, só pedidos semânticos):** clipes H.264 pequenos (640×360, 48
quadros, código de barras do número do quadro; +100 no segundo projeto), proxy gerado pelo próprio editor (altura 180).

| Cenário | Prova |
|---|---|
| perda no meio do scrub / do refino / com o timer de refino pendente / com o playhead andando / com o monitor oculto | convergência nos 3 sentidos (estado do host, hash na tela == `render.frame` original, barcode), `context_generation` +1 exato; no timer pendente o refino disparou com o contexto ainda perdido (`refined: 2`) e recuperou |
| 6 perdas seguidas | `context_generation` 5→11, `gl_textures` 2→2, RSS do editor 173,8→173,8 MB (**só o editor**) |
| ocultar/mostrar, 8 alternâncias com o playhead andando | `on_screen` volta, sem falha, hash/barcode do quadro final |
| diálogo sobre o monitor (`flow.import` → `dialog.cancel`) | `on_screen` false enquanto aberto, volta ao fechar, posição converge |
| janela minimizada e ativada | converge (a minimização **não** altera `on_screen`: não é observável como ocultar) |
| perda + troca de projeto antes da restauração | o segundo projeto desenha o próprio quadro (barcode 130), sessão sem assets do primeiro, proxy do segundo |
| contexto **nunca** restaurado | `Falha: context_lost` no cabeçalho, nova intenção responde em ~1 s sem travar, `monitor.reload` recupera (`context_generation` 0 na página nova) |
| perda + reload antes da restauração | nenhum timer da página velha afeta a nova; proxy e convergência no quadro seguinte |
| `monitor.reload` com `render.frame` explícito 1920×1080 em voo | a corrida aconteceu de fato (resposta `cancelled` tipada, 0,0 s); o reload é repetido até acontecer, tentativas registradas no JSON |
| `project.open` com explícito em voo | `cancelled` tipado, não pendurado; o projeto novo e o antigo reabrem com proxy próprio |
| export original antes e depois de tudo | 12 quadros, `framemd5` **idêntico** (worker Node+Chrome servindo o `dist` real) |

Capturas reais da janela (`--shots`, 6 PNGs; duas foram abertas e conferidas, `hidden-loss-reshown` e `second-project`; removidas na limpeza): por exemplo o monitor
reexibido após perda com o monitor oculto mostra o quadro 33 (00:00:01:09) correto, e o segundo projeto mostra `b.mp4` no quadro 15.

## Limitações

- A perda é **forçada** com `WEBGL_lose_context` (eventos reais `webglcontextlost/restored` no WKWebView, disparados pelo
  `debug` do runtime a partir de um `index.html` de rascunho em `TESSEL_RUNTIME_DIR`; o `runtime.js` é o mesmo). Não é reset de GPU
  nem queda de driver/processo do WebKit.
- Sem timing de GPU ou de foto na tela; nada de latência foi medido aqui.
- Só o RSS do editor foi medido; os processos auxiliares do WebKit não são filhos dele e não entram.
- Uma tela, macOS, build release, datasets pequenos; sem 4K, sem soak, sem outros displays.
- Overlays que não sejam diálogo (popover/tooltip) e foco do teclado sobre a WebView continuam **P** (sem comando semântico).
- `gl_textures` de originais cresce com os quadros distintos visitados (4→15 numa sessão de varredura) até o teto de 256 MiB;
  é a retenção desenhada (LRU por bytes), e zera/recua com reload, troca de projeto, perda e `release`.
- O harness depende de `ffmpeg`, Chrome (export) e do binário release já construído (`--binary`); exige 6 GiB livres.

## Observações do Rust (sem correção)

Nenhuma prova falhou, então nenhuma mudança foi feita. Leitura de `crates/core/src/session.rs`, para quem for mexer:

- `ContextLost`/`Closed` limpam assets, proxies e recusas e falham o quadro, mas **não** limpam `refine` nem `presented`;
- um refino pode disparar com o contexto ainda perdido: falha com `context_lost` e se recupera por `needs_redraw` na restauração
  (visto na prova do timer pendente);
- `presented` fica velho enquanto o status é `Failed`.

Nada disso produziu imagem errada ou estado preso nas provas. Se a frente A/coordenador decidir limpar `refine`/`presented` na
perda, o teste deve nascer em `crates/core/src/session/tests.rs` e a validação **não foi rodada aqui** (Cargo é da frente A):

```bash
./scripts/cargo test -p tessel-editor-core --locked
./scripts/cargo test -p tessel-editor --locked viewport_host_tests
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy --workspace --all-targets --locked
```

O binário usado pelo harness (`target/release/tessel-editor`, 09:43) não foi reconstruído por mim. O harness roda contra qualquer
binário com `--binary` e deve ser re-executado pelo coordenador depois de qualquer rebuild de Rust.

## Linha proposta para a matriz (`docs/features/README.md`, F021)

A linha de F021 já é `V⁵ | V | V | V`. Nenhuma coluna muda; sugiro acrescentar ao rodapé ⁵ (sem alterar os V):

> Ciclo de vida sob incidente (perda de contexto com proxy, ocultar/mostrar, diálogo, reload/troca de projeto com quadro em voo,
> export intacto): [relatório](../viewport-lifecycle-report.md), `scripts/viewport-lifecycle-proof.py` (janela real, perda forçada)
> e `runtime/test/lifecycle.test.mjs`. UI: V (macOS, release, uma tela; barcode/hash, sem foto na tela). CLI headless: N/A (sem
> janela; o runtime headless coberto pelos 8 testes de Chrome). Lib: V (runtime + sessão existente; sem mudança de Rust).

## Reproduzir

```bash
cd runtime && pnpm run build && TESSEL_REQUIRE_RUNTIME=1 pnpm test
python3 scripts/viewport-lifecycle-proof.py --out /tmp/lifecycle.json            # tudo
python3 scripts/viewport-lifecycle-proof.py --only losses,never_restored --shots /tmp/shots
```

`--only` aceita `export,losses,hidden_loss,cycles,hide_show,dialog,window,loss_switch,switch,never_restored,loss_reload,reload`
(o baseline sempre roda). Os artefatos próprios ficam em `/tmp/tvl-*` e são removidos, salvo `--keep`.

## Revalidação do coordenador

O coordenador repetiu a suíte com `TESSEL_REQUIRE_RUNTIME=1 pnpm test`: **61/61**,
zero falhas e zero skips. Repetiu também o harness completo na janela WKWebView real:
**147 PASS, 0 FAIL**, com export de 12 quadros idêntico antes/depois dos incidentes.
Sem `--shots`: esta repetição prova hashes/código de barras/estado, sem inspeção visual adicional.

Resultado bruto compacto e hashes do binário, bundle runtime e harness:
[`window-proof.json`](viewport-performance-results/lifecycle/window-proof.json) e
[`PROVENANCE.json`](viewport-performance-results/lifecycle/PROVENANCE.json).
O commit de origem do binário release não é conhecido; foi usado o binário existente,
sem recompilar Rust. Portanto não é aceite do futuro build integrado da migração.

A revisão acrescentou limpeza das sequências temporárias com `t.after`, inclusive se
o teste falhar, e fechamento do runtime se o setup do asset lançar erro. Depois desse
ajuste foram repetidos os 8 testes de lifecycle; o runtime de produto não mudou.
A liberação observada é a dos contadores Three.js/cache, não uma medida da memória física
da GPU ou do driver. Nenhum processo preexistente foi encerrado pelo coordenador.
